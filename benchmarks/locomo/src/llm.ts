import type { QACategory } from './types'

import { env } from 'node:process'
import { setTimeout as sleep } from 'node:timers/promises'

import { generateObject } from '@xsai/generate-object'
import { z } from 'zod'

const DEFAULT_MAX_ATTEMPTS = 4
const RETRY_BASE_DELAY_MS = 1_500
const RETRY_MAX_DELAY_MS = 10_000
const RETRYABLE_ERROR_CODES = new Set([
  'ECONNREFUSED',
  'ECONNRESET',
  'ETIMEDOUT',
  'UND_ERR_BODY_TIMEOUT',
  'UND_ERR_CONNECT_TIMEOUT',
  'UND_ERR_HEADERS_TIMEOUT',
  'UND_ERR_SOCKET',
])

const SYSTEM_PROMPT = [
  'You answer questions using only the provided conversation context.',
].join('\n')

const getCategoryGuidance = (category: QACategory): string => {
  switch (category) {
    case 1:
      return [
        'Question type: multi-hop.',
        '- Scan all relevant memories before answering.',
        '- The answer may require multiple facts or multiple items; include every required piece.',
        '- If several candidate items are topically similar, include only items that satisfy the exact person, event, relation, and time scope in the question.',
        '- Keep the final answer concise, but do not drop required parts just to make it shorter.',
      ].join('\n')
    case 2:
      return [
        'Question type: temporal.',
        '- Match the specific person, event, and time scope asked in the question.',
        '- Use the memory or conversation date as the anchor for relative dates.',
        '- Convert relative time references to the best absolute date, month, or year.',
        '- Do not prefer the most recent memory unless the question asks for the current or latest state.',
        '- If the evidence only supports an approximate date, answer with the best supported approximate date.',
      ].join('\n')
    case 3:
      return [
        'Question type: open-domain.',
        '- Answer the underlying question using only supported information from the context.',
        '- A slightly longer answer is acceptable when needed to capture the full idea.',
        '- Do not invent conclusions that are not supported by the context.',
        '- If evidence is insufficient, say so briefly.',
      ].join('\n')
    case 4:
      return [
        'Question type: single-hop.',
        '- Find the single directly supported fact or entity.',
        '- Prefer the shortest exact span from the context when it answers the question.',
        '- Do not answer with a full sentence if a short entity, object, place, title, or value is sufficient.',
        '- If multiple candidate memories match the topic, choose the one matching the exact person, object, event, and time scope in the question.',
      ].join('\n')
    case 5:
      return [
        'Question type: adversarial.',
        '- Answer only if the provided context contains direct support.',
        '- If not supported, answer: No information available.',
      ].join('\n')
  }
}

const buildPrompt = (context: string, question: string, category: QACategory): string => {
  const contextSection = context.length > 0
    ? context
    : '(empty)'

  return `Use only the provided conversation context.
Answer the question with a concise phrase.
Prefer exact words from the context when they directly answer the question.
If multiple candidate memories match the topic, choose the one matching the person, event, and time scope in the question.
If two plausible answers remain, answer both briefly.
Do not explain your reasoning.

${getCategoryGuidance(category)}

Context:
${contextSection}

Question: ${question}`
}

const ANSWER_SCHEMA = z.object({
  answer: z.string().describe('The concise answer phrase. Do not include reasoning.'),
})

const getErrorCode = (error: unknown): string | undefined => {
  if (error == null || typeof error !== 'object')
    return undefined

  if ('code' in error && typeof error.code === 'string')
    return error.code

  if ('cause' in error)
    return getErrorCode(error.cause)

  return undefined
}

const getErrorMessage = (error: unknown): string => {
  if (error instanceof Error)
    return error.message
  return String(error)
}

const isRetryableGenerateError = (error: unknown): boolean => {
  const code = getErrorCode(error)
  if (code != null && RETRYABLE_ERROR_CODES.has(code))
    return true

  const message = getErrorMessage(error)
  return message.includes('fetch failed')
    || message.includes('terminated')
    || message.includes('timeout')
}

const summarizeQuestion = (question: string): string =>
  question.length <= 80 ? question : `${question.slice(0, 77)}...`

/**
 * Generate an answer for a single QA pair.
 */
export const generateAnswer = async (
  context: string,
  question: string,
  category: QACategory,
  model = 'gpt-4o-mini',
  seed?: number,
): Promise<string> => {
  const prompt = buildPrompt(context, question, category)
  let lastError: unknown

  for (let attempt = 1; attempt <= DEFAULT_MAX_ATTEMPTS; attempt++) {
    try {
      const { object } = await generateObject({
        apiKey: env.OPENAI_API_KEY ?? '',
        baseURL: env.OPENAI_BASE_URL ?? 'https://api.openai.com/v1',
        messages: [
          { content: SYSTEM_PROMPT, role: 'system' },
          { content: prompt, role: 'user' },
        ],
        model,
        output: 'object' as const,
        reasoningEffort: 'none',
        schema: ANSWER_SCHEMA,
        schemaDescription: 'A concise answer for a LoCoMo question.',
        schemaName: 'locomo_answer',
        seed,
        strict: true,
        temperature: 0,
      })

      return object.answer.trim()
    }
    catch (error) {
      lastError = error
      if (!isRetryableGenerateError(error) || attempt === DEFAULT_MAX_ATTEMPTS)
        break

      const delayMs = Math.min(RETRY_BASE_DELAY_MS * 2 ** (attempt - 1), RETRY_MAX_DELAY_MS)
      const code = getErrorCode(error) ?? 'UNKNOWN'
      console.warn(
        `generateAnswer failed for "${summarizeQuestion(question)}" `
        + `(attempt ${attempt}/${DEFAULT_MAX_ATTEMPTS}, code=${code}); retrying in ${delayMs}ms`,
      )
      await sleep(delayMs)
    }
  }

  const code = getErrorCode(lastError) ?? 'UNKNOWN'
  const message = getErrorMessage(lastError)
  throw new Error(
    `generateAnswer failed for "${summarizeQuestion(question)}" `
    + `after ${DEFAULT_MAX_ATTEMPTS} attempts (code=${code}): ${message}`,
  )
}
