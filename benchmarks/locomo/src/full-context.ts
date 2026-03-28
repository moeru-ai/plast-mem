import type { DialogTurn, LoCoMoSample } from './types'

import { getOrderedSessions } from './ingest'

const formatTurn = (turn: DialogTurn): string => {
  const lines = [`${turn.speaker} said, "${turn.text}"`]

  if (turn.blip_caption != null && turn.blip_caption.length > 0)
    lines.push(`and shared ${turn.blip_caption}.`)

  if (turn.search_query != null && turn.search_query.length > 0)
    lines.push(`Search query: ${turn.search_query}.`)

  return `${lines.join('\n')}\n`
}

const formatSession = (
  sample: LoCoMoSample,
  sessionIndex: number,
  sessionDate: Date | null,
  turns: DialogTurn[],
): string => {
  const rawDate = sample.conversation[`session_${sessionIndex}_date_time`]
  const dateLine = typeof rawDate === 'string'
    ? rawDate
    : (sessionDate?.toISOString() ?? 'unknown date')

  let body = `DATE: ${dateLine}\nCONVERSATION:\n`
  for (const turn of turns)
    body += formatTurn(turn)

  return `${body.trimEnd()}\n`
}

export const buildFullContext = (
  sample: LoCoMoSample,
  question: string,
): string => {
  const sessions = getOrderedSessions(sample)
  const parts: string[] = []
  for (let index = 0; index < sessions.length; index++) {
    const session = sessions[index]
    parts.push(formatSession(sample, index + 1, session.date, session.turns).trimEnd())
  }

  const promptPrefix = 'Below is a conversation between two people over multiple days.\n\n'
  const fullContext = `${promptPrefix}${parts.join('\n\n')}`.trim()
  if (fullContext.length === 0)
    throw new Error(`Full context is empty for sample ${sample.sample_id} question "${question}"`)
  return fullContext
}
