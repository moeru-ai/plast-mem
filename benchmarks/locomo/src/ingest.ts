import type { DialogTurn, LoCoMoSample } from './types'

import process from 'node:process'

import { readFileSync, writeFileSync } from 'node:fs'

import { addMessage } from 'plastmem'
import { v7 as uuidv7 } from 'uuid'

const INGEST_DELAY_MS = 100

const buildSpeakerRoleMap = (sample: LoCoMoSample): Map<string, 'assistant' | 'user'> => {
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

const getOrderedSessions = (sample: LoCoMoSample): Array<{ turns: DialogTurn[] }> => {
  const sessions: Array<{ turns: DialogTurn[] }> = []
  for (let sn = 1; sn <= 100; sn++) {
    const turns = sample.conversation[`session_${sn}`]
    if (!Array.isArray(turns))
      break
    sessions.push({ turns })
  }
  return sessions
}

const ingestSample = async (
  sample: LoCoMoSample,
  conversationId: string,
  baseUrl: string,
  onProgress?: (done: number, total: number) => void,
): Promise<void> => {
  const roleMap = buildSpeakerRoleMap(sample)
  const sessions = getOrderedSessions(sample)

  let totalTurns = 0
  for (const s of sessions) totalTurns += s.turns.length

  let done = 0

  for (const session of sessions) {
    for (const turn of session.turns) {
      if (!turn.text.trim()) {
        done++
        continue
      }

      const role = roleMap.get(turn.speaker) ?? 'user'

      await addMessage({
        baseUrl,
        body: {
          conversation_id: conversationId,
          message: { content: turn.text, role },
        },
      })

      done++
      onProgress?.(done, totalTurns)

      if (INGEST_DELAY_MS > 0) {
        await new Promise<void>((resolve) => {
          const timer = setTimeout(resolve, INGEST_DELAY_MS)
          void timer
        })
      }
    }
  }
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
