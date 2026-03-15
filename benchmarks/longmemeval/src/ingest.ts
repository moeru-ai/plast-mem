import type { BenchmarkAddMessages } from 'plastmem'

import type { LongMemEvalSample, LongMemEvalTurn } from './types'

import { readFile, writeFile } from 'node:fs/promises'

import { uuid } from '@insel-null/uuid'
import { benchmarkAddMessages } from 'plastmem'

const TURN_INTERVAL_MINS = 1

interface BatchMessage {
  content: string
  role: LongMemEvalTurn['role']
  timestamp?: string
}

type ConversationIdMap = Record<string, string>

const parseSessionDate = (dateStr: string): Date | null => {
  const timestamp = Date.parse(dateStr)
  if (Number.isNaN(timestamp))
    return null
  return new Date(timestamp)
}

const getTurnTimestamp = (sessionDate: Date | null, turnIndex: number): string | undefined => {
  if (sessionDate == null)
    return undefined

  return new Date(sessionDate.getTime() + turnIndex * TURN_INTERVAL_MINS * 60 * 1000).toISOString()
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

export const ingestSample = async (
  sample: LongMemEvalSample,
  conversationId: string,
  baseUrl: string,
): Promise<void> => {
  const messages = buildMessages(sample)
  await addMessagesInBulk(baseUrl, conversationId, messages)
}

export const ingestAll = async (
  samples: LongMemEvalSample[],
  baseUrl: string,
): Promise<ConversationIdMap> => {
  const ids: ConversationIdMap = {}

  for (const sample of samples) {
    const conversationId = uuid.v7()
    ids[sample.question_id] = conversationId
    await ingestSample(sample, conversationId, baseUrl)
  }

  return ids
}

export const loadConversationIds = async (path: string): Promise<ConversationIdMap> => {
  try {
    const content = await readFile(path, 'utf-8')
    return JSON.parse(content) as ConversationIdMap
  }
  catch {
    return {}
  }
}

export const saveConversationIds = async (path: string, ids: ConversationIdMap): Promise<void> => {
  await writeFile(path, JSON.stringify(ids, null, 2))
}
