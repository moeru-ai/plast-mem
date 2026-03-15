import { setTimeout as sleep } from 'node:timers/promises'

import { retrieveMemory } from 'plastmem'

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

const isRetryableFetchError = (error: unknown): boolean => {
  const code = getErrorCode(error)
  if (code != null && RETRYABLE_ERROR_CODES.has(code))
    return true

  const message = getErrorMessage(error)
  return message.includes('fetch failed') || message.includes('timeout')
}

const summarizeQuestion = (question: string): string =>
  question.length <= 80 ? question : `${question.slice(0, 77)}...`

export const getContext = async (
  conversationId: string,
  question: string,
  baseUrl: string,
): Promise<string> => {
  let lastError: unknown

  for (let attempt = 1; attempt <= DEFAULT_MAX_ATTEMPTS; attempt++) {
    try {
      const res = await retrieveMemory({
        baseUrl,
        body: {
          conversation_id: conversationId,
          episodic_limit: 10,
          query: question,
          semantic_limit: 5,
        },
        throwOnError: true,
      })
      return res.data ?? ''
    }
    catch (error) {
      lastError = error
      if (!isRetryableFetchError(error) || attempt === DEFAULT_MAX_ATTEMPTS)
        break

      const delayMs = Math.min(RETRY_BASE_DELAY_MS * 2 ** (attempt - 1), RETRY_MAX_DELAY_MS)
      const code = getErrorCode(error) ?? 'UNKNOWN'
      console.warn(
        `retrieveMemory timeout for ${conversationId} "${summarizeQuestion(question)}" `
        + `(attempt ${attempt}/${DEFAULT_MAX_ATTEMPTS}, code=${code}); retrying in ${delayMs}ms`,
      )
      await sleep(delayMs)
    }
  }

  const code = getErrorCode(lastError) ?? 'UNKNOWN'
  const message = getErrorMessage(lastError)
  throw new Error(
    `retrieveMemory failed for ${conversationId} "${summarizeQuestion(question)}" `
    + `after ${DEFAULT_MAX_ATTEMPTS} attempts (code=${code}): ${message}`,
  )
}
