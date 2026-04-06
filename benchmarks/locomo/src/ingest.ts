import type { ImportMessages } from 'plastmem'

import type { DialogTurn, LoCoMoSample } from './types'

import { progress as createProgress, log } from '@clack/prompts'
import { uuid } from '@insel-null/uuid'
import { messagesImport } from 'plastmem'

import { runWithConcurrency } from './concurrency'
import { waitForAll } from './wait'

// Minutes between consecutive turns within a session
const TURN_INTERVAL_MINS = 1
export interface OrderedSession { date: Date | null, turns: DialogTurn[] }

interface BatchMessage {
  content: string
  role: string
  timestamp?: number
}

const appendImageContext = (parts: string[], caption?: string, query?: string): void => {
  const normalizedCaption = caption?.trim()
  if (normalizedCaption == null || normalizedCaption.length === 0)
    return

  const normalizedQuery = query?.trim()
  if (normalizedQuery != null && normalizedQuery.length > 0) {
    parts.push(`[Image: ${normalizedCaption}; Image Retrieval Keywords: ${normalizedQuery}]`)
    return
  }

  parts.push(`[Image: ${normalizedCaption}]`)
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

const importConversation = async (
  baseUrl: string,
  conversationId: string,
  messages: BatchMessage[],
): Promise<void> => {
  await messagesImport({
    baseUrl,
    body: {
      conversation_id: conversationId,
      eof: true,
      messages: messages as unknown as ImportMessages['messages'],
    },
    throwOnError: true,
  })
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

const getTurnTimestampMs = (sessionDate: Date | null, turnIndex: number): number | undefined => {
  if (sessionDate == null)
    return undefined

  const timestamp = new Date(sessionDate.getTime() + turnIndex * TURN_INTERVAL_MINS * 60 * 1000)
  return timestamp.getTime()
}

const buildMessages = (sample: LoCoMoSample): BatchMessage[] => {
  const sessions = getOrderedSessions(sample)
  const messages: BatchMessage[] = []

  for (const session of sessions) {
    for (let i = 0; i < session.turns.length; i++) {
      const turn = session.turns[i]
      if (turn == null || turn.text.trim().length === 0)
        continue

      const timestamp = getTurnTimestampMs(session.date, i)
      const parts = [turn.text.trim()]
      appendImageContext(parts, turn.blip_caption, turn.query ?? turn.search_query)
      messages.push({
        content: parts.join(' '),
        role: turn.speaker.trim() || 'User',
        ...(timestamp != null ? { timestamp } : {}),
      })
    }
  }

  return messages
}

const getSampleLabel = (sampleId: string): string => `Sample ${sampleId}`

const getSampleDebugLabel = (sampleId: string, conversationId: string): string =>
  `Sample ${sampleId} (${conversationId})`

const ingestSample = async (
  sample: LoCoMoSample,
  conversationId: string,
  baseUrl: string,
  onProgress?: (done: number, total: number) => void,
): Promise<void> => {
  const messages = buildMessages(sample)
  const totalMessages = messages.length
  onProgress?.(0, totalMessages)
  await importConversation(baseUrl, conversationId, messages)
  onProgress?.(totalMessages, totalMessages)
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
      log.info(`Reusing ${getSampleLabel(sample.sample_id)}`)
      return
    }

    const conversationId = uuid.v7()
    const totalMessages = buildMessages(sample).length
    const progress = createProgress({ max: Math.max(totalMessages, 1) })
    progress.start(`${getSampleDebugLabel(sample.sample_id, conversationId)} ingesting 0/${totalMessages}`)
    let advanced = 0
    await ingestSample(sample, conversationId, baseUrl, (done, total) => {
      const delta = done - advanced
      advanced = done
      if (delta > 0)
        progress.advance(delta, `${getSampleLabel(sample.sample_id)} ingesting ${done}/${total}`)
    })
    if (settleAndFlushAfterSampleIngest) {
      progress.stop(`${getSampleLabel(sample.sample_id)} ingested ${totalMessages} messages`)
      await waitForAll([conversationId], baseUrl)
    }
    else {
      progress.stop(`${getSampleLabel(sample.sample_id)} ingested`)
    }
    ids[sample.sample_id] = conversationId
    if (onSampleComplete != null) {
      persistChain = persistChain.then(async () => onSampleComplete({ ...ids }))
      await persistChain
    }
  })

  await runWithConcurrency(tasks, concurrency)

  return ids
}
