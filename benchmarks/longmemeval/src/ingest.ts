import type { AddMessage, AddMessageResult, BenchmarkJobStatus } from 'plastmem'

import type { LongMemEvalSample, LongMemEvalTurn } from './types'

import { progress as createProgress } from '@clack/prompts'
import { sleep } from '@moeru/std'
import { addMessage, benchmarkFlush, benchmarkJobStatus } from 'plastmem'

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

const isBackpressured = (value: unknown): value is AddMessageResult =>
  typeof value === 'object'
  && value !== null
  && 'accepted' in value
  && (value as { accepted: unknown }).accepted === false
  && (!('reason' in value) || (value as { reason?: unknown }).reason === 'backpressure')

const getStatus = async (
  baseUrl: string,
  conversationId: string,
): Promise<BenchmarkJobStatus> => {
  const res = await benchmarkJobStatus({
    baseUrl,
    query: { conversation_id: conversationId },
    throwOnError: true,
  })

  return res.data
}

const waitUntilConversationAdmissible = async (
  baseUrl: string,
  conversationId: string,
): Promise<void> => {
  while (true) {
    const status = await getStatus(baseUrl, conversationId)
    if (status.admissible_for_add)
      return

    await sleep(ADMISSION_POLL_INTERVAL_MS)
  }
}

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

const flushConversationTail = async (
  baseUrl: string,
  conversationId: string,
): Promise<void> => {
  while (true) {
    const status = await getStatus(baseUrl, conversationId)

    if (status.flushable) {
      await benchmarkFlush({
        baseUrl,
        body: { conversation_id: conversationId },
        throwOnError: true,
      })
      return
    }

    if (status.messages_pending === 0 && !status.fence_active && status.segmentation_jobs_active === 0)
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

  let done = 0
  for (const message of messages) {
    while (true) {
      const accepted = await sendMessage(baseUrl, conversationId, message)
      if (accepted) {
        done++
        onProgress?.(done, total)
        break
      }

      await waitUntilConversationAdmissible(baseUrl, conversationId)
    }
  }

  await flushConversationTail(baseUrl, conversationId)
}

export const ingestSampleWithProgress = async (
  sample: LongMemEvalSample,
  conversationId: string,
  baseUrl: string,
): Promise<void> => {
  const total = buildMessages(sample).length
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
