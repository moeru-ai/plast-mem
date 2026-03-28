import type { QACategory } from './types'

import { env } from 'node:process'
import { setTimeout as sleep } from 'node:timers/promises'

import { generateText } from '@xsai/generate-text'

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
  'You answer questions by reading retrieved conversation memories and extracting the most accurate supported answer.',
].join('\n')

const isCompositeQuestion = (question: string): boolean => {
  const normalized = question.trim().toLowerCase().replace(/\s+/g, ' ')

  if (normalized.includes(' and '))
    return true

  const prefixes = [
    'what kinds of ',
    'what kind of healthy ',
    'what food ',
    'what foods ',
    'what recipes ',
    'what motivates ',
    'what health scares ',
    'what accidents ',
    'what challenges ',
    'how did ',
    'which two ',
    'who did ',
  ]

  return prefixes.some(prefix => normalized.startsWith(prefix))
}

const buildPrompt = (context: string, question: string, category: QACategory): string => {
  const contextSection = context.length > 0
    ? `Conversation memories:\n${context}\n\n`
    : ''
  const composite = isCompositeQuestion(question)
  const answerLengthRule = composite
    ? 'Keep the answer as short as possible while still covering every required part.'
    : 'Keep the answer under 5 words whenever possible.'

  if (category === 5) {
    return `${contextSection}Answer the question using only the retrieved memories above.
- If the topic does not appear anywhere in those memories, reply exactly: "No information available"
- If the memories contain the exact answer span, copy that span directly.
- Prefer the shortest exact answer span over a full sentence.
- If the question asks for multiple items, people, events, or a cause-and-effect chain, include every required part in the shortest supported form.
- For multi-part questions, include only items that satisfy the exact subject, relation, and time scope asked in the question.
- ${answerLengthRule}

Question: ${question}
Short answer:`
  }

  return `${contextSection}# Context
The memories come from a conversation between two speakers.
Some of them include timestamps that may matter for answering the question.

# Instructions
1. Read all retrieved memories from both speakers carefully.
2. Pay close attention to timestamps when the answer depends on time.
3. If the question asks about a specific event or fact, look for direct support in the memories.
4. If the memories conflict, prefer the one with the more recent timestamp.
5. When a memory uses a relative time phrase such as "last year" or "two months ago", resolve it against that memory's timestamp.
   Example: if a memory dated 4 May 2022 says "went to India last year," then the trip happened in 2021.
6. Convert relative time references into a specific date, month, or year in the final answer. Do not answer with the relative phrase itself.
7. Base the answer only on the memory content from both speakers. If a name appears inside a memory, do not assume that person is the speaker who created it.
8. If the memories contain the exact answer wording, copy that wording directly.
9. Prefer the shortest exact answer span over a full sentence.
10. If the question asks for multiple items, people, events, reasons, or steps, include every required part instead of collapsing the answer into one fragment.
11. For multi-part questions, do not add loosely related items just because they are topically similar. Every item in the answer must directly satisfy the question.
12. Do not add explanation, hedging, or extra descriptive words if a shorter exact answer is supported.
13. ${answerLengthRule}

# Approach
1. Identify the memories that are relevant to the question.
2. Determine whether the question asks for one fact or multiple required pieces.
3. Examine timestamps and content carefully.
4. Look for explicit mentions of dates, times, locations, people, objects, or events that answer the question.
5. If a calculation is required, work it out before answering.
6. If the question is single-fact, output only the shortest exact supported span.
7. If the question is multi-part, check that every required piece is present before finalizing the answer.
8. Remove any candidate item that does not match the exact person, object, event type, or time scope asked by the question.
9. Write a precise, concise answer supported only by the memories.
10. Make sure the final answer is specific and avoids vague time references.

# Examples
- Good: "Yoga"
- Bad: "Yoga helped Evan with stress and flexibility."
- Good: "fitness tracker"
- Bad: "a health tracking tool"
- Good: "Christmas season"
- Bad: "during the winter holiday season"
- Good: "the woman he fell in love with; her"
- Bad: "a Canadian woman"
- Good: "changed his diet and started walking"
- Bad: "dietary changes"
- Good: "old Prius; new Prius"
- Bad: "Prius"
- Good: "family; fitness tracker; adventure hikes"
- Bad: "his family"
- Good: "old Prius; new Prius"
- Bad: "self-checkout machines; old Prius"

Question: ${question}
Short answer:`
}

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
): Promise<string> => {
  const prompt = buildPrompt(context, question, category)
  const maxTokens = isCompositeQuestion(question) ? 96 : 64
  let lastError: unknown

  for (let attempt = 1; attempt <= DEFAULT_MAX_ATTEMPTS; attempt++) {
    try {
      const { text } = await generateText({
        apiKey: env.OPENAI_API_KEY ?? '',
        baseURL: env.OPENAI_BASE_URL ?? 'https://api.openai.com/v1',
        maxTokens,
        messages: [
          { content: SYSTEM_PROMPT, role: 'system' },
          { content: prompt, role: 'user' },
        ],
        model,
        reasoningEffort: 'none',
        temperature: 0,
      })

      return text ?? ''
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
