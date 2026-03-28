import type { AddMessage } from 'plastmem'

import type { DialogTurn, LoCoMoSample } from './types'

import { readFile, writeFile } from 'node:fs/promises'
import { stdout } from 'node:process'

import { uuid } from '@insel-null/uuid'
import { Spinner } from 'picospinner'
import { addMessage } from 'plastmem'

import { flushConversationTailWhenReady, waitUntilConversationAdmissible } from './wait'

// Minutes between consecutive turns within a session
const TURN_INTERVAL_MINS = 1
export interface OrderedSession { date: Date | null, turns: DialogTurn[] }

interface BatchMessage {
  content: string
  role: string
  timestamp?: number
}

const runWithConcurrency = async (
  tasks: Array<() => Promise<void>>,
  concurrency: number,
): Promise<void> => {
  if (tasks.length === 0)
    return

  const limit = Math.max(1, Math.floor(concurrency))
  let nextIndex = 0

  const worker = async (): Promise<void> => {
    while (true) {
      const taskIndex = nextIndex
      nextIndex += 1
      if (taskIndex >= tasks.length)
        return
      await tasks[taskIndex]()
    }
  }

  await Promise.all(
    Array.from({ length: Math.min(limit, tasks.length) }).fill(0).map(async () => worker()),
  )
}

const SESSION_DATE_RE = /^(\d{1,2}):(\d{2})\s*(am|pm)\s+on\s+(\d{1,2})\s+(\w+),\s+(\d{4})$/i
const MONTH_INDEX_BY_NAME: Record<string, number> = {
  april: 3,
  august: 7,
  december: 11,
  february: 1,
  january: 0,
  july: 6,
  june: 5,
  march: 2,
  may: 4,
  november: 10,
  october: 9,
  september: 8,
}

/**
 * Parse LoCoMo session date strings like "1:56 pm on 8 May, 2023" into a UTC Date.
 * Returns null if the string cannot be parsed.
 */
const parseSessionDate = (dateStr: string): Date | null => {
  const match = SESSION_DATE_RE.exec(dateStr)
  if (match == null)
    return null
  const [, hStr, minStr, meridiem, dStr, monthStr, yearStr] = match
  const monthIndex = MONTH_INDEX_BY_NAME[monthStr.toLowerCase()]
  if (monthIndex == null)
    return null
  let hours = Number.parseInt(hStr, 10)
  const mins = Number.parseInt(minStr, 10)
  if (meridiem.toLowerCase() === 'pm' && hours !== 12)
    hours += 12
  if (meridiem.toLowerCase() === 'am' && hours === 12)
    hours = 0
  return new Date(Date.UTC(Number.parseInt(yearStr, 10), monthIndex, Number.parseInt(dStr, 10), hours, mins))
}

interface AddMessageResult {
  accepted: boolean
  reason?: string
}

const isBackpressured = (value: unknown): value is AddMessageResult =>
  typeof value === 'object'
  && value !== null
  && 'accepted' in value
  && (value as { accepted: unknown }).accepted === false
  && (!('reason' in value) || (value as { reason?: unknown }).reason === 'backpressure')

const sendMessage = async (
  baseUrl: string,
  conversationId: string,
  message: BatchMessage,
): Promise<boolean> => {
  const res = await addMessage({
    baseUrl,
    body: {
      conversation_id: conversationId,
      message: message as unknown as AddMessage['message'],
    },
    throwOnError: false,
  })

  if (res.response?.ok)
    return true

  if (res.response?.status === 429 && isBackpressured(res.error))
    return false

  const status = res.response?.status ?? 'network'
  throw new Error(`addMessage failed with status ${status}`)
}

export const getOrderedSessions = (sample: LoCoMoSample): OrderedSession[] => {
  const sessions: OrderedSession[] = []
  for (let sn = 1; sn <= 100; sn++) {
    const turns = sample.conversation[`session_${sn}`]
    if (!Array.isArray(turns))
      break
    const dateStr = sample.conversation[`session_${sn}_date_time`]
    const date = typeof dateStr === 'string' ? parseSessionDate(dateStr) : null
    sessions.push({ date, turns })
  }
  return sessions
}

const countTotalTurns = (sessions: OrderedSession[]): number =>
  sessions.reduce((total, session) => total + session.turns.length, 0)

const getTurnTimestampMs = (sessionDate: Date | null, turnIndex: number): number | undefined => {
  if (sessionDate == null)
    return undefined

  const timestamp = new Date(sessionDate.getTime() + turnIndex * TURN_INTERVAL_MINS * 60 * 1000)
  return timestamp.getTime()
}

const ingestSample = async (
  sample: LoCoMoSample,
  conversationId: string,
  baseUrl: string,
  onProgress?: (done: number, total: number) => void,
): Promise<void> => {
  const sessions = getOrderedSessions(sample)
  const totalTurns = countTotalTurns(sessions)
  const messages: BatchMessage[] = []

  let done = 0

  for (const session of sessions) {
    for (let i = 0; i < session.turns.length; i++) {
      const turn = session.turns[i]
      done++
      if (turn == null || turn.text.trim().length === 0)
        continue

      const timestamp = getTurnTimestampMs(session.date, i)
      messages.push({
        content: turn.text,
        role: turn.speaker.trim() || 'User',
        ...(timestamp != null ? { timestamp } : {}),
      })
      onProgress?.(done, totalTurns)
    }
  }

  for (const message of messages) {
    while (true) {
      const accepted = await sendMessage(baseUrl, conversationId, message)
      if (accepted)
        break

      await waitUntilConversationAdmissible(baseUrl, conversationId)
    }
  }
}

export const ingestAll = async (
  samples: LoCoMoSample[],
  existingIds: Record<string, string>,
  baseUrl: string,
  concurrency: number,
  settleAndFlushAfterSampleIngest: boolean,
  onSampleComplete?: (ids: Record<string, string>) => Promise<void>,
): Promise<Record<string, string>> => {
  const ids: Record<string, string> = { ...existingIds }
  let persistChain = Promise.resolve()

  const tasks = samples.map(sample => async () => {
    const existingConversationId = ids[sample.sample_id]
    if (existingConversationId != null && existingConversationId.length > 0) {
      stdout.write(`  Reusing sample ${sample.sample_id} (${existingConversationId})\n`)
      return
    }

    const conversationId = uuid.v7()
    stdout.write(`  Ingesting sample ${sample.sample_id} (${conversationId})\n`)
    const spinner = new Spinner(`Ingesting sample ${sample.sample_id}`)
    let lastPct = 0
    await ingestSample(sample, conversationId, baseUrl, (done, total) => {
      const pct = Math.floor((done / total) * 100)
      if (pct >= lastPct + 20) {
        spinner.setText(`Ingesting sample ${sample.sample_id} (${conversationId}) ${pct}%`)
        lastPct = pct
      }
    })
    if (settleAndFlushAfterSampleIngest) {
      spinner.setText(`Waiting for episodic settle on sample ${sample.sample_id} (${conversationId})`)
      const flushed = await flushConversationTailWhenReady(baseUrl, conversationId)
      if (flushed)
        spinner.setText(`Flushed episodic tail for sample ${sample.sample_id} (${conversationId})`)
    }
    ids[sample.sample_id] = conversationId
    if (onSampleComplete != null) {
      persistChain = persistChain.then(async () => onSampleComplete({ ...ids }))
      await persistChain
    }
    spinner.succeed(`Ingested sample ${sample.sample_id} (${conversationId})`)
  })

  await runWithConcurrency(tasks, concurrency)

  return ids
}

export const loadConversationIds = async (path: string): Promise<Record<string, string>> => {
  try {
    const content = await readFile(path, 'utf-8')
    return JSON.parse(content) as Record<string, string>
  }
  catch {
    return {}
  }
}

export const saveConversationIds = async (path: string, ids: Record<string, string>): Promise<void> => {
  await writeFile(path, JSON.stringify(ids, null, 2))
}
