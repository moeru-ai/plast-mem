import { appendFile, mkdir, readFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { env } from 'node:process'
import { setTimeout as sleep } from 'node:timers/promises'
import { fileURLToPath } from 'node:url'

import { embed } from '@xsai/embed'
import { retrieveMemory } from 'plastmem'

const __dirname = dirname(fileURLToPath(import.meta.url))

const DEFAULT_MAX_ATTEMPTS = 4
const RETRY_BASE_DELAY_MS = 1_500
const RETRY_MAX_DELAY_MS = 10_000
const EMBEDDING_DIM = 1024
const QUERY_EMBEDDING_CACHE_PATH = resolve(__dirname, '../data/query_embedding_cache.jsonl')

const RETRYABLE_ERROR_CODES = new Set([
  'ECONNREFUSED',
  'ECONNRESET',
  'ETIMEDOUT',
  'UND_ERR_BODY_TIMEOUT',
  'UND_ERR_CONNECT_TIMEOUT',
  'UND_ERR_HEADERS_TIMEOUT',
  'UND_ERR_SOCKET',
])

const queryEmbeddingCacheState: {
  promise: null | Promise<Map<string, number[]>>
} = {
  promise: null,
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

const isRetryableFetchError = (error: unknown): boolean => {
  const code = getErrorCode(error)
  if (code != null && RETRYABLE_ERROR_CODES.has(code))
    return true

  const message = getErrorMessage(error)
  return message.includes('fetch failed') || message.includes('timeout')
}

const summarizeQuestion = (question: string): string =>
  question.length <= 80 ? question : `${question.slice(0, 77)}...`

const getEmbeddingBaseUrl = (): string =>
  (env.OPENAI_BASE_URL ?? 'https://api.openai.com/v1').replace(/\/$/, '')

const getEmbeddingModel = (): string =>
  env.OPENAI_EMBEDDING_MODEL ?? 'text-embedding-3-small'

const getQueryEmbeddingCacheKey = (question: string): string =>
  `${getEmbeddingBaseUrl()}|${getEmbeddingModel()}|${question}`

const loadQueryEmbeddingCache = async (): Promise<Map<string, number[]>> => {
  try {
    const raw = await readFile(QUERY_EMBEDDING_CACHE_PATH, 'utf-8')
    const cache = new Map<string, number[]>()

    for (const line of raw.split('\n')) {
      const trimmed = line.trim()
      if (trimmed.length === 0)
        continue

      const parsed = JSON.parse(trimmed) as { embedding: number[], key: string }
      cache.set(parsed.key, parsed.embedding)
    }

    return cache
  }
  catch {
    return new Map<string, number[]>()
  }
}

const getQueryEmbeddingCache = async (): Promise<Map<string, number[]>> => {
  queryEmbeddingCacheState.promise ??= loadQueryEmbeddingCache()
  return queryEmbeddingCacheState.promise
}

const appendQueryEmbeddingCacheEntry = async (
  key: string,
  embedding: number[],
): Promise<void> => {
  const line = `${JSON.stringify({ embedding, key })}\n`
  await mkdir(dirname(QUERY_EMBEDDING_CACHE_PATH), { recursive: true })
  await appendFile(QUERY_EMBEDDING_CACHE_PATH, line)
}

const fetchQueryEmbedding = async (question: string): Promise<number[]> => {
  const apiKey = env.OPENAI_API_KEY ?? ''
  if (apiKey.length === 0)
    throw new Error('OPENAI_API_KEY not set for benchmark embedding cache')

  const result = await embed({
    apiKey,
    baseURL: getEmbeddingBaseUrl(),
    dimensions: EMBEDDING_DIM,
    input: question,
    model: getEmbeddingModel(),
  })

  return result.embedding
}

const getQueryEmbedding = async (question: string): Promise<number[]> => {
  const key = getQueryEmbeddingCacheKey(question)
  const cache = await getQueryEmbeddingCache()
  const cached = cache.get(key)
  if (cached != null)
    return cached

  const embedding = await fetchQueryEmbedding(question)
  cache.set(key, embedding)
  await appendQueryEmbeddingCacheEntry(key, embedding)
  return embedding
}

export const getContext = async (
  conversationId: string,
  question: string,
  baseUrl: string,
): Promise<string> => {
  let lastError: unknown

  for (let attempt = 1; attempt <= DEFAULT_MAX_ATTEMPTS; attempt++) {
    try {
      const queryEmbedding = await getQueryEmbedding(question)
      const body = {
        conversation_id: conversationId,
        episodic_limit: 10,
        query: question,
        query_embedding: queryEmbedding,
        semantic_limit: 20,
      }
      const res = await retrieveMemory({
        baseUrl,
        body,
        throwOnError: true,
      })
      const context = res.data ?? ''
      if (context.length === 0) {
        throw new Error(
          `retrieveMemory returned empty context for ${conversationId} "${summarizeQuestion(question)}"`,
        )
      }
      return context
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
