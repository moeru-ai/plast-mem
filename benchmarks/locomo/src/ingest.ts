import type { BenchmarkAddMessages } from 'plastmem'

import type { DialogTurn, LoCoMoSample } from './types'

import process from 'node:process'

import { readFileSync, writeFileSync } from 'node:fs'

import { benchmarkAddMessages } from 'plastmem'
import { v7 as uuidv7 } from 'uuid'

// Minutes between consecutive turns within a session
const TURN_INTERVAL_MINS = 1
interface BatchMessage {
  content: string
  role: SpeakerRole
  timestamp?: number
}
interface OrderedSession { date: Date | null, turns: DialogTurn[] }
type SpeakerRole = 'assistant' | 'user'

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

const addMessagesInBulk = async (
  baseUrl: string,
  conversationId: string,
  messages: BatchMessage[],
): Promise<void> => {
  await benchmarkAddMessages({
    baseUrl,
    body: {
      conversation_id: conversationId,
      force_process: true,
      messages: messages as unknown as BenchmarkAddMessages['messages'],
    },
    throwOnError: true,
  })
}

const buildSpeakerRoleMap = (sample: LoCoMoSample): Map<string, SpeakerRole> => {
  const speakers: string[] = []
  let n = 1
  while (speakers.length < 2) {
    const turns = sample.conversation[`session_${n}`]
    if (!Array.isArray(turns))
      break
    for (const turn of turns) {
      if (!speakers.includes(turn.speaker))
        speakers.push(turn.speaker)
    }
    n++
  }
  const map = new Map<string, 'assistant' | 'user'>()
  if (speakers.length > 0)
    map.set(speakers[0], 'user')
  if (speakers.length > 1)
    map.set(speakers[1], 'assistant')
  return map
}

const getOrderedSessions = (sample: LoCoMoSample): OrderedSession[] => {
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
  const roleMap = buildSpeakerRoleMap(sample)
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

      const role = roleMap.get(turn.speaker) ?? 'user'
      const timestamp = getTurnTimestampMs(session.date, i)
      messages.push({
        content: turn.text,
        role,
        ...(timestamp != null ? { timestamp } : {}),
      })
      onProgress?.(done, totalTurns)
    }
  }

  await addMessagesInBulk(baseUrl, conversationId, messages)
}

export const ingestAll = async (
  samples: LoCoMoSample[],
  baseUrl: string,
): Promise<Record<string, string>> => {
  const ids: Record<string, string> = {}

  for (const sample of samples) {
    const conversationId = uuidv7()
    ids[sample.sample_id] = conversationId

    process.stdout.write(`  Ingesting sample ${sample.sample_id} (${conversationId})...`)
    let lastPct = 0
    await ingestSample(sample, conversationId, baseUrl, (done, total) => {
      const pct = Math.floor((done / total) * 100)
      if (pct >= lastPct + 20) {
        process.stdout.write(` ${pct}%`)
        lastPct = pct
      }
    })
    process.stdout.write(' done\n')
  }

  return ids
}

export const loadConversationIds = (path: string): Record<string, string> => {
  try {
    return JSON.parse(readFileSync(path, 'utf-8')) as Record<string, string>
  }
  catch {
    return {}
  }
}

export const saveConversationIds = (path: string, ids: Record<string, string>): void => {
  writeFileSync(path, JSON.stringify(ids, null, 2))
}
