import type {
  LongMemEvalQuestionType,
  LongMemEvalResult,
  LongMemEvalSample,
} from './types'

import { createHash } from 'node:crypto'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { dirname } from 'node:path'

import z from 'zod'

const CHECKPOINT_VERSION = 1
const JSON_FILE_RE = /\.json$/i

const questionTypeSchema = z.enum([
  'knowledge-update',
  'multi-session',
  'single-session-assistant',
  'single-session-preference',
  'single-session-user',
  'temporal-reasoning',
])

const resultSchema = z.object({
  context: z.string(),
  conversation_id: z.string(),
  gold_answer: z.string(),
  prediction: z.string(),
  question: z.string(),
  question_id: z.string(),
  question_type: questionTypeSchema,
  score: z.union([z.literal(0), z.literal(1)]),
  verdict: z.string(),
})

const runConfigSchema = z.object({
  baseUrl: z.string(),
  dataFile: z.string(),
  datasetName: z.string(),
  model: z.string(),
  outFile: z.string(),
  questionTypes: z.array(questionTypeSchema),
  sampleIds: z.array(z.string()),
  seed: z.number().int().optional(),
  waitForBackground: z.boolean(),
})

const sampleCheckpointSchema = z.object({
  conversation_id: z.string().nullable(),
  error: z.string().nullable(),
  ingest_done: z.boolean(),
  question_id: z.string(),
  result: resultSchema.nullable(),
  status: z.enum(['complete', 'failed', 'pending', 'running']),
})

const runCheckpointSchema = z.object({
  completed_at: z.string().nullable(),
  config: runConfigSchema,
  fingerprint: z.string(),
  samples: z.record(z.string(), sampleCheckpointSchema),
  started_at: z.string(),
  updated_at: z.string(),
  version: z.literal(CHECKPOINT_VERSION),
})

export interface BenchmarkRunConfig {
  baseUrl: string
  dataFile: string
  datasetName: string
  model: string
  outFile: string
  questionTypes: LongMemEvalQuestionType[]
  sampleIds: string[]
  seed?: number
  waitForBackground: boolean
}

export interface RunCheckpoint {
  completed_at: null | string
  config: BenchmarkRunConfig
  fingerprint: string
  samples: Record<string, SampleCheckpoint>
  started_at: string
  updated_at: string
  version: 1
}

export interface SampleCheckpoint {
  conversation_id: null | string
  error: null | string
  ingest_done: boolean
  question_id: string
  result: LongMemEvalResult | null
  status: 'complete' | 'failed' | 'pending' | 'running'
}

const normalizeConfig = (config: BenchmarkRunConfig): string => JSON.stringify({
  baseUrl: config.baseUrl,
  dataFile: config.dataFile,
  datasetName: config.datasetName,
  model: config.model,
  questionTypes: config.questionTypes.toSorted((left, right) => left.localeCompare(right)),
  sampleIds: config.sampleIds.toSorted((left, right) => left.localeCompare(right)),
  seed: config.seed,
  waitForBackground: config.waitForBackground,
})

export const buildCheckpointFingerprint = (config: BenchmarkRunConfig): string =>
  createHash('sha256').update(normalizeConfig(config)).digest('hex')

export const buildCheckpointPath = (outFile: string): string =>
  outFile.replace(JSON_FILE_RE, '.checkpoint.json')

export const createCheckpoint = (
  config: BenchmarkRunConfig,
  dataset: LongMemEvalSample[],
): RunCheckpoint => ({
  completed_at: null,
  config,
  fingerprint: buildCheckpointFingerprint(config),
  samples: Object.fromEntries(dataset.map(sample => [
    sample.question_id,
    {
      conversation_id: null,
      error: null,
      ingest_done: false,
      question_id: sample.question_id,
      result: null,
      status: 'pending',
    },
  ])),
  started_at: new Date().toISOString(),
  updated_at: new Date().toISOString(),
  version: CHECKPOINT_VERSION,
})

export const saveCheckpoint = async (
  path: string,
  checkpoint: RunCheckpoint,
): Promise<void> => {
  checkpoint.updated_at = new Date().toISOString()
  await mkdir(dirname(path), { recursive: true })
  await writeFile(path, JSON.stringify(checkpoint, null, 2))
}

export const loadCheckpoint = async (path: string): Promise<null | RunCheckpoint> => {
  try {
    const raw = await readFile(path, 'utf-8')
    return runCheckpointSchema.parse(JSON.parse(raw)) as RunCheckpoint
  }
  catch {
    return null
  }
}

export const collectResults = (checkpoint: RunCheckpoint): LongMemEvalResult[] =>
  Object.values(checkpoint.samples)
    .flatMap(sample => sample.result == null ? [] : [sample.result])
    .toSorted((left, right) => left.question_id.localeCompare(right.question_id))
