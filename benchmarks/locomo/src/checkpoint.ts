import type {
  BenchmarkVariant,
  BenchmarkVariantOutput,
  LoCoMoSample,
} from './types'

import { createHash } from 'node:crypto'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { dirname, join } from 'node:path'

import {
  parseRunManifest,
  parseSampleResult,
  parseSampleState,
} from './schemas'

const RUN_MANIFEST_VERSION = 2
const RUN_MANIFEST_FILE = 'run.json'
const OVERALL_JSON_FILE = 'overall.json'
const OVERALL_MARKDOWN_FILE = 'overall.md'
const SAMPLE_RESULTS_DIR = 'samples'

export interface BenchmarkRunConfig {
  baseUrl: string
  compareFullContext: boolean
  dataFile: string
  model: string
  outDir: string
  sampleConcurrency: number
  sampleIds: string[]
  seed?: number
  useLlmJudge: boolean
  waitForBackground: boolean
}

export interface RunManifest {
  completed_at: null | string
  config: BenchmarkRunConfig
  fingerprint: string
  sample_ids: string[]
  started_at: string
  updated_at: string
  version: 2
}

export interface SampleResultFile {
  sample_id: string
  variants: Partial<Record<BenchmarkVariant, BenchmarkVariantOutput>>
}

export interface SampleState {
  conversation_id: null | string
  error: null | string
  ingest_done: boolean
  sample_id: string
  status: 'complete' | 'failed' | 'pending' | 'running'
  updated_at: string
  variants: Partial<Record<BenchmarkVariant, VariantState>>
}

export interface VariantState {
  eval_done: boolean
}

const createVariantState = (): VariantState => ({
  eval_done: false,
})

const normalizeConfig = (config: BenchmarkRunConfig): string => JSON.stringify({
  baseUrl: config.baseUrl,
  compareFullContext: config.compareFullContext,
  dataFile: config.dataFile,
  model: config.model,
  sampleConcurrency: config.sampleConcurrency,
  sampleIds: config.sampleIds.toSorted((left, right) => left.localeCompare(right)),
  seed: config.seed,
  useLlmJudge: config.useLlmJudge,
  waitForBackground: config.waitForBackground,
})

export const buildRunFingerprint = (config: BenchmarkRunConfig): string =>
  createHash('sha256').update(normalizeConfig(config)).digest('hex')

export const buildRunManifestPath = (outDir: string): string =>
  join(outDir, RUN_MANIFEST_FILE)

export const buildOverallJsonPath = (outDir: string): string =>
  join(outDir, OVERALL_JSON_FILE)

export const buildOverallMarkdownPath = (outDir: string): string =>
  join(outDir, OVERALL_MARKDOWN_FILE)

export const buildSampleStatePath = (outDir: string, sampleId: string): string =>
  join(outDir, SAMPLE_RESULTS_DIR, `${sampleId}.state.json`)

export const buildSampleResultPath = (outDir: string, sampleId: string): string =>
  join(outDir, SAMPLE_RESULTS_DIR, `${sampleId}.json`)

export const createRunManifest = (
  config: BenchmarkRunConfig,
  samples: LoCoMoSample[],
): RunManifest => ({
  completed_at: null,
  config,
  fingerprint: buildRunFingerprint(config),
  sample_ids: samples.map(sample => sample.sample_id),
  started_at: new Date().toISOString(),
  updated_at: new Date().toISOString(),
  version: RUN_MANIFEST_VERSION,
})

export const createSampleState = (
  sample: LoCoMoSample,
  compareFullContext: boolean,
): SampleState => ({
  conversation_id: null,
  error: null,
  ingest_done: false,
  sample_id: sample.sample_id,
  status: 'pending',
  updated_at: new Date().toISOString(),
  variants: {
    plastmem: createVariantState(),
    ...(compareFullContext ? { full_context: createVariantState() } : {}),
  },
})

export const saveRunManifest = async (
  path: string,
  manifest: RunManifest,
): Promise<void> => {
  manifest.updated_at = new Date().toISOString()
  await mkdir(dirname(path), { recursive: true })
  await writeFile(path, JSON.stringify(manifest, null, 2))
}

export const saveSampleState = async (
  outDir: string,
  sampleState: SampleState,
): Promise<void> => {
  sampleState.updated_at = new Date().toISOString()
  const path = buildSampleStatePath(outDir, sampleState.sample_id)
  await mkdir(dirname(path), { recursive: true })
  await writeFile(path, JSON.stringify(sampleState, null, 2))
}

export const saveSampleResult = async (
  outDir: string,
  result: SampleResultFile,
): Promise<void> => {
  const path = buildSampleResultPath(outDir, result.sample_id)
  await mkdir(dirname(path), { recursive: true })
  await writeFile(path, JSON.stringify(result, null, 2))
}

export const loadRunManifest = async (path: string): Promise<null | RunManifest> => {
  try {
    const raw = await readFile(path, 'utf-8')
    return parseRunManifest(JSON.parse(raw))
  }
  catch {
    return null
  }
}

export const loadSampleState = async (
  outDir: string,
  sampleId: string,
): Promise<null | SampleState> => {
  try {
    const raw = await readFile(buildSampleStatePath(outDir, sampleId), 'utf-8')
    return parseSampleState(JSON.parse(raw))
  }
  catch {
    return null
  }
}

export const loadSampleResult = async (
  outDir: string,
  sampleId: string,
): Promise<null | SampleResultFile> => {
  try {
    const raw = await readFile(buildSampleResultPath(outDir, sampleId), 'utf-8')
    return parseSampleResult(JSON.parse(raw))
  }
  catch {
    return null
  }
}

export const ensureSampleStates = async (
  outDir: string,
  samples: LoCoMoSample[],
  compareFullContext: boolean,
): Promise<Record<string, SampleState>> => {
  const states = await Promise.all(samples.map(async (sample) => {
    const existing = await loadSampleState(outDir, sample.sample_id)
    if (existing != null)
      return [sample.sample_id, existing] as const

    const created = createSampleState(sample, compareFullContext)
    await saveSampleState(outDir, created)
    return [sample.sample_id, created] as const
  }))

  return Object.fromEntries(states)
}

export const createEmptySampleResult = (sampleId: string): SampleResultFile => ({
  sample_id: sampleId,
  variants: {},
})

export const getVariantOrder = (compareFullContext: boolean): BenchmarkVariant[] =>
  compareFullContext ? ['plastmem', 'full_context'] : ['plastmem']
