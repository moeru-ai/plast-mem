import type {
  BenchmarkRunConfig,
  BenchmarkVariant,
  LoCoMoSample,
  RunCheckpoint,
  SampleCheckpoint,
  VariantCheckpoint,
} from './types'

import { createHash } from 'node:crypto'
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import { dirname } from 'node:path'

const CHECKPOINT_VERSION = 1
const JSON_FILE_RE = /\.json$/i

const createVariantCheckpoint = (): VariantCheckpoint => ({
  eval_done: false,
  results: [],
  score_done: false,
})

const createSampleCheckpoint = (
  sample: LoCoMoSample,
  compareFullContext: boolean,
): SampleCheckpoint => ({
  conversation_id: null,
  error: null,
  ingest_done: false,
  sample_id: sample.sample_id,
  status: 'pending',
  variants: {
    plastmem: createVariantCheckpoint(),
    ...(compareFullContext ? { full_context: createVariantCheckpoint() } : {}),
  },
})

const normalizeConfig = (config: BenchmarkRunConfig): string => JSON.stringify({
  baseUrl: config.baseUrl,
  compareFullContext: config.compareFullContext,
  concurrency: config.concurrency,
  dataFile: config.dataFile,
  model: config.model,
  sampleIds: config.sampleIds.toSorted((left, right) => left.localeCompare(right)),
  useLlmJudge: config.useLlmJudge,
  waitForBackground: config.waitForBackground,
})

export const buildCheckpointFingerprint = (config: BenchmarkRunConfig): string =>
  createHash('sha256').update(normalizeConfig(config)).digest('hex')

export const buildCheckpointPath = (outFile: string): string =>
  outFile.replace(JSON_FILE_RE, '.checkpoint.json')

export const createCheckpoint = (
  config: BenchmarkRunConfig,
  samples: LoCoMoSample[],
): RunCheckpoint => ({
  completed_at: null,
  config,
  fingerprint: buildCheckpointFingerprint(config),
  samples: Object.fromEntries(samples.map(sample => [
    sample.sample_id,
    createSampleCheckpoint(sample, config.compareFullContext),
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
    return JSON.parse(raw) as RunCheckpoint
  }
  catch {
    return null
  }
}

export const resetCheckpointFile = async (path: string): Promise<void> => {
  await rm(path, { force: true })
}

export const isCheckpointCompatible = (
  checkpoint: RunCheckpoint,
  config: BenchmarkRunConfig,
): boolean =>
  checkpoint.version === CHECKPOINT_VERSION
  && checkpoint.fingerprint === buildCheckpointFingerprint(config)

export const getVariantOrder = (compareFullContext: boolean): BenchmarkVariant[] =>
  compareFullContext ? ['plastmem', 'full_context'] : ['plastmem']
