import type { BenchmarkRunConfig, RunManifest, SampleState } from './checkpoint'
import type { LoCoMoSample } from './types'

import { readdir, readFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { cwd, env, exit, loadEnvFile } from 'node:process'
import { fileURLToPath } from 'node:url'

import {
  cancel,
  confirm,
  intro,
  isCancel,
  log,
  multiselect,
  note,
  outro,
  select,
} from '@clack/prompts'

import {
  buildRunManifestPath,
  createRunManifest,
  ensureSampleStates,
  loadRunManifest,
  loadSampleState,
  saveRunManifest,
} from './checkpoint'
import { printFinalSummary, runBenchmark } from './runner'
import { parseLoCoMoSamples } from './schemas'

const DEFAULT_DATA_FILE = resolve(cwd(), 'data/locomo10.json')
const RESULTS_DIR = resolve(cwd(), 'results')
const COLON_DOT_RE = /[:.]/g
const LONG_CONTEXT_SAMPLE_IDS = ['conv-43', 'conv-47', 'conv-48'] as const
const MINIMAL_SAMPLE_IDS = ['conv-42', 'conv-48'] as const
const DEFAULT_SAMPLE_IDS = ['conv-42', 'conv-44', 'conv-48', 'conv-50'] as const
const TRAILING_SLASH_RE = /\/$/
const DEFAULT_SAMPLE_CONCURRENCY = 4
const __dirname = dirname(fileURLToPath(import.meta.url))
const WORKSPACE_ROOT = resolve(__dirname, '../../../')

const prompt = async <T>(value: Promise<symbol | T>): Promise<T> => {
  const resolved = await value
  if (isCancel(resolved)) {
    cancel('Benchmark cancelled.')
    exit(0)
  }
  return resolved as T
}

const timestampedOutputDir = (): string =>
  resolve(cwd(), `results/${new Date().toISOString().replace(COLON_DOT_RE, '-')}`)

const loadSamples = async (dataFile: string): Promise<LoCoMoSample[]> => {
  const raw = await readFile(dataFile, 'utf-8')
  return parseLoCoMoSamples(JSON.parse(raw))
}

const loadDefaultSamples = async (): Promise<LoCoMoSample[]> => {
  try {
    return await loadSamples(DEFAULT_DATA_FILE)
  }
  catch (error) {
    if ((error as NodeJS.ErrnoException).code !== 'ENOENT')
      throw error

    throw new Error(
      `LoCoMo dataset not found at ${DEFAULT_DATA_FILE}.\n`
      + 'Download it with:\n'
      + `curl -L https://github.com/snap-research/locomo/raw/main/data/locomo10.json --create-dirs -o ${DEFAULT_DATA_FILE}`,
    )
  }
}

const getRequiredChatModel = (): string => {
  const model = env.OPENAI_CHAT_MODEL?.trim()
  if (model == null || model.length === 0) {
    throw new Error(
      'OPENAI_CHAT_MODEL not set in the root .env.\n'
      + 'Set it before running the benchmark; the CLI does not prompt for a model.',
    )
  }
  return model
}

const getOptionalChatSeed = (): number | undefined => {
  const rawSeed = env.OPENAI_CHAT_SEED?.trim()
  if (rawSeed == null || rawSeed.length === 0)
    return undefined

  if (!/^-?\d+$/.test(rawSeed))
    return undefined

  return Number.parseInt(rawSeed, 10)
}

const resolvePresetSampleIds = (
  allSampleIds: string[],
  preset: readonly string[],
): string[] =>
  allSampleIds.filter(sampleId => preset.includes(sampleId))

const getLatestRunManifestPath = async (): Promise<null | string> => {
  try {
    const entries = await readdir(RESULTS_DIR, { withFileTypes: true })
    const runDirs = entries
      .filter(entry => entry.isDirectory())
      .map(entry => entry.name)
      .toSorted((left, right) => right.localeCompare(left))

    for (const runDir of runDirs) {
      const manifestPath = buildRunManifestPath(resolve(RESULTS_DIR, runDir))
      const manifest = await loadRunManifest(manifestPath)
      if (manifest != null && manifest.completed_at == null)
        return manifestPath
    }

    return null
  }
  catch {
    return null
  }
}

const describeRun = async (manifest: RunManifest): Promise<string> => {
  const states = await Promise.all(
    manifest.sample_ids.map(async sampleId => loadSampleState(manifest.config.outDir, sampleId)),
  )

  const complete = states.filter(sample => sample?.status === 'complete').length
  const failed = states.filter(sample => sample?.status === 'failed').length
  const running = states.filter(sample => sample?.status === 'running').length
  const pending = manifest.sample_ids.length - complete - failed - running

  return [
    `Started: ${manifest.started_at}`,
    `Updated: ${manifest.updated_at}`,
    `Run: ${manifest.config.outDir}`,
    `Complete: ${complete}`,
    `Failed: ${failed}`,
    `Running: ${running}`,
    `Pending: ${pending}`,
  ].join('\n')
}

const loadLatestRun = async (): Promise<null | {
  manifest: RunManifest
  manifestPath: string
  sampleStates: Record<string, SampleState>
}> => {
  const manifestPath = await getLatestRunManifestPath()
  if (manifestPath == null)
    return null

  const manifest = await loadRunManifest(manifestPath)
  if (manifest == null || manifest.completed_at != null)
    return null

  note(await describeRun(manifest), 'Latest run found')
  const shouldResume = await prompt<boolean>(confirm({
    initialValue: true,
    message: 'Resume from the latest run?',
  }))

  if (!shouldResume)
    return null

  const allSamples = await loadDefaultSamples()
  const samples = allSamples.filter(sample => manifest.config.sampleIds.includes(sample.sample_id))
  const sampleStates = await ensureSampleStates(
    manifest.config.outDir,
    samples,
    manifest.config.compareFullContext,
  )

  return { manifest, manifestPath, sampleStates }
}

const promptForConfig = async (): Promise<BenchmarkRunConfig> => {
  const defaultBaseUrl = (env.PLASTMEM_BASE_URL ?? 'http://localhost:3000').replace(TRAILING_SLASH_RE, '')
  const defaultModel = getRequiredChatModel()
  const defaultSeed = getOptionalChatSeed()
  const allSamples = await loadDefaultSamples()
  const allSampleIds = allSamples.map(sample => sample.sample_id)
  const sampleMode = await prompt<string>(select({
    initialValue: 'recommended',
    message: 'Which samples should run?',
    options: [
      { label: 'Minimal subset (42/48)', value: 'minimal' },
      { label: 'Recommended subset (42/44/48/50)', value: 'recommended' },
      { label: 'Long-context subset (43/47/48)', value: 'long_context' },
      { label: 'All samples', value: 'all' },
      { label: 'Custom selection', value: 'custom' },
    ],
  }))

  const selectedSampleIds = sampleMode === 'all'
    ? allSampleIds
    : sampleMode === 'minimal'
      ? resolvePresetSampleIds(allSampleIds, MINIMAL_SAMPLE_IDS)
      : sampleMode === 'recommended'
        ? resolvePresetSampleIds(allSampleIds, DEFAULT_SAMPLE_IDS)
        : sampleMode === 'long_context'
          ? resolvePresetSampleIds(allSampleIds, LONG_CONTEXT_SAMPLE_IDS)
          : await prompt<string[]>(multiselect({
              initialValues: [],
              message: 'Choose sample IDs',
              options: allSamples.map(sample => ({
                label: sample.sample_id,
                value: sample.sample_id,
              })),
              required: true,
            }))

  const compareMode = await prompt<string>(select({
    initialValue: 'plastmem',
    message: 'Comparison mode',
    options: [
      { label: 'plast-mem only', value: 'plastmem' },
      { label: 'plast-mem + full-context', value: 'compare' },
    ],
  }))

  const useLlmJudge = await prompt<boolean>(confirm({
    initialValue: false,
    message: 'Enable LLM judge scoring?',
  }))

  return {
    baseUrl: defaultBaseUrl,
    compareFullContext: compareMode === 'compare',
    dataFile: DEFAULT_DATA_FILE,
    model: defaultModel,
    outDir: timestampedOutputDir(),
    sampleConcurrency: DEFAULT_SAMPLE_CONCURRENCY,
    sampleIds: selectedSampleIds.toSorted((left, right) => left.localeCompare(right)),
    seed: defaultSeed,
    useLlmJudge,
    waitForBackground: true,
  }
}

const prepareRun = async (
  config: BenchmarkRunConfig,
  samples: LoCoMoSample[],
): Promise<{
  manifest: RunManifest
  manifestPath: string
  sampleStates: Record<string, SampleState>
}> => {
  const manifest = createRunManifest(config, samples)
  const manifestPath = buildRunManifestPath(config.outDir)
  await saveRunManifest(manifestPath, manifest)
  const sampleStates = await ensureSampleStates(config.outDir, samples, config.compareFullContext)

  return {
    manifest,
    manifestPath,
    sampleStates,
  }
}

const main = async (): Promise<void> => {
  try {
    loadEnvFile(resolve(WORKSPACE_ROOT, '.env'))
  }
  catch { }

  intro('LoCoMo Benchmark')

  const resumed = await loadLatestRun()
  const config = resumed?.manifest.config ?? await promptForConfig()

  if (config.useLlmJudge && (env.OPENAI_API_KEY == null || env.OPENAI_API_KEY.length === 0)) {
    cancel('OPENAI_API_KEY not set for LLM judge mode.')
    exit(1)
  }

  const allSamples = await loadDefaultSamples()
  const samples = allSamples.filter(sample => config.sampleIds.includes(sample.sample_id))
  if (samples.length === 0) {
    cancel('No samples selected.')
    exit(1)
  }

  const {
    manifest,
    manifestPath,
    sampleStates,
  } = resumed ?? await prepareRun(config, samples)

  note([
    `data: ${config.dataFile}`,
    `run: ${config.outDir}`,
    `manifest: ${manifestPath}`,
    `samples: ${samples.length}`,
    `sampleConcurrency: ${config.sampleConcurrency}`,
    `model: ${config.model}`,
    `seed: ${config.seed ?? 'unset'}`,
    `baseUrl: ${config.baseUrl}`,
    `llmJudge: ${config.useLlmJudge ? 'on' : 'off'}`,
    `compare: ${config.compareFullContext ? 'plast-mem + full-context' : 'plast-mem only'}`,
  ].join('\n'), 'Run configuration')

  log.step('Running selected samples')
  const completedRun = await runBenchmark(manifest, sampleStates, samples)
  log.success('Benchmark run finished')

  printFinalSummary(completedRun.output)
  outro(`Results written to ${completedRun.manifest.config.outDir}`)
}

// eslint-disable-next-line @masknet/no-top-level
main().catch((error) => {
  console.error(error)
  exit(1)
})
