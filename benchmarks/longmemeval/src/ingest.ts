import type { ImportMessages, SegmentationStateStatus } from 'plastmem'

import type { LongMemEvalSample, LongMemEvalTurn } from './types'

import { progress as createProgress } from '@clack/prompts'
import { sleep } from '@moeru/std'
import { messagesImport, segmentationState } from 'plastmem'

const TURN_INTERVAL_MINS = 1
const ADMISSION_POLL_INTERVAL_MS = 1_000

interface BatchMessage {
  content: string
  role: LongMemEvalTurn['role']
  timestamp?: number
}

const parseSessionDate = (dateStr: string): Date | null => {
  const timestamp = Date.parse(dateStr)
  if (Number.isNaN(timestamp))
    return null
  return new Date(timestamp)
}

const getTurnTimestamp = (sessionDate: Date | null, turnIndex: number): number | undefined => {
  if (sessionDate == null)
    return undefined

  return sessionDate.getTime() + turnIndex * TURN_INTERVAL_MINS * 60 * 1000
}

const buildMessages = (sample: LongMemEvalSample): BatchMessage[] => {
  const messages: BatchMessage[] = []

  for (const [sessionIndex, turns] of sample.haystack_sessions.entries()) {
    const sessionDate = parseSessionDate(sample.haystack_dates[sessionIndex] ?? '')

    for (const [turnIndex, turn] of turns.entries()) {
      if (turn.content.trim().length === 0)
        continue

      const timestamp = getTurnTimestamp(sessionDate, turnIndex)
      messages.push({
        content: turn.content,
        role: turn.role,
        ...(timestamp != null ? { timestamp } : {}),
      })
    }
  }

  return messages
}

const getStatus = async (
  baseUrl: string,
  conversationId: string,
): Promise<SegmentationStateStatus> => {
  const res = await segmentationState({
    baseUrl,
    query: { conversation_id: conversationId },
    throwOnError: true,
  })

  return res.data
}

const waitUntilConversationSettled = async (
  baseUrl: string,
  conversationId: string,
): Promise<void> => {
  while (true) {
    const status = await getStatus(baseUrl, conversationId)

    if (status.done)
      return

    await sleep(ADMISSION_POLL_INTERVAL_MS)
  }
}

export const ingestSample = async (
  sample: LongMemEvalSample,
  conversationId: string,
  baseUrl: string,
  onProgress?: (done: number, total: number) => void,
): Promise<void> => {
  const messages = buildMessages(sample)
  const total = messages.length

  onProgress?.(0, total)
  await messagesImport({
    baseUrl,
    body: {
      conversation_id: conversationId,
      eof: true,
      messages: messages as unknown as ImportMessages['messages'],
    },
    throwOnError: true,
  })
  onProgress?.(total, total)
  await waitUntilConversationSettled(baseUrl, conversationId)
}

export const countSampleMessages = (sample: LongMemEvalSample): number =>
  buildMessages(sample).length

export const ingestSampleWithProgress = async (
  sample: LongMemEvalSample,
  conversationId: string,
  baseUrl: string,
): Promise<void> => {
  const total = countSampleMessages(sample)
  const progress = createProgress({ max: Math.max(total, 1) })
  progress.start(`Ingesting ${sample.question_id} 0/${total}`)

  let advanced = 0
  try {
    await ingestSample(sample, conversationId, baseUrl, (done, count) => {
      const delta = done - advanced
      advanced = done
      if (delta > 0)
        progress.advance(delta, `Ingesting ${sample.question_id} ${done}/${count}`)
    })
    progress.stop(`Ingested ${sample.question_id} ${total}/${total}`)
  }
  catch (error) {
    progress.stop(`Ingest failed ${sample.question_id} ${advanced}/${total}`)
    throw error
  }
}
