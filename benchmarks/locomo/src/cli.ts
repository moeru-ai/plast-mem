import type { BenchmarkRunConfig, RunCheckpoint } from './checkpoint'
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
  text,
} from '@clack/prompts'

import {
  buildCheckpointPath,
  createCheckpoint,
  loadCheckpoint,
} from './checkpoint'
import { printFinalSummary, runBenchmark } from './runner'
import { parseLoCoMoSamples } from './schemas'

const __dirname = dirname(fileURLToPath(import.meta.url))

const DEFAULT_DATA_FILE = resolve(cwd(), 'data/locomo10.json')
const RESULTS_DIR = resolve(cwd(), 'results')
const CHECKPOINT_FILE_SUFFIX = '.checkpoint.json'
const COLON_DOT_RE = /[:.]/g
const LONG_CONTEXT_SAMPLE_IDS = ['conv-43', 'conv-47', 'conv-48'] as const
const MINIMAL_SAMPLE_IDS = ['conv-42', 'conv-48'] as const
const DEFAULT_SAMPLE_IDS = ['conv-42', 'conv-44', 'conv-48', 'conv-50'] as const
const TRAILING_SLASH_RE = /\/$/

const prompt = async <T>(value: Promise<symbol | T>): Promise<T> => {
  const resolved = await value
  if (isCancel(resolved)) {
    cancel('Benchmark cancelled.')
    exit(0)
  }
  return resolved as T
}

const timestampedOutputPath = (): string =>
  resolve(cwd(), `results/${new Date().toISOString().replace(COLON_DOT_RE, '-')}.json`)

const loadSamples = async (dataFile: string): Promise<LoCoMoSample[]> => {
  const raw = await readFile(dataFile, 'utf-8')
  return parseLoCoMoSamples(JSON.parse(raw))
}

const loadDefaultSamples = async (): Promise<LoCoMoSample[]> => {
  try {
    return await loadSamples(DEFAULT_DATA_FILE)
  }
  catch {
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
      + 'Set it before running the benchmark.',
    )
  }
  return model
}

const resolvePresetSampleIds = (
  allSampleIds: string[],
  preset: readonly string[],
): string[] =>
  allSampleIds.filter(sampleId => preset.includes(sampleId))

const getLatestCheckpointPath = async (): Promise<null | string> => {
  try {
    const entries = await readdir(RESULTS_DIR, { withFileTypes: true })
    const checkpointNames = entries
      .filter(entry => entry.isFile() && entry.name.endsWith(CHECKPOINT_FILE_SUFFIX))
      .map(entry => entry.name)
      .toSorted((left, right) => right.localeCompare(left))

    const latest = checkpointNames[0]
    if (latest == null)
      return null

    return resolve(RESULTS_DIR, latest)
  }
  catch {
    return null
  }
}

const describeCheckpoint = (checkpoint: RunCheckpoint): string => {
  const samples = Object.values(checkpoint.samples)
  const complete = samples.filter(sample => sample.status === 'complete').length
  const failed = samples.filter(sample => sample.status === 'failed').length
  const running = samples.filter(sample => sample.status === 'running').length
  const pending = samples.length - complete - failed - running
  return [
    `Started: ${checkpoint.started_at}`,
    `Updated: ${checkpoint.updated_at}`,
    `Complete: ${complete}`,
    `Failed: ${failed}`,
    `Running: ${running}`,
    `Pending: ${pending}`,
  ].join('\n')
}

const loadLatestCheckpoint = async (): Promise<null | { checkpoint: RunCheckpoint, checkpointPath: string }> => {
  const checkpointPath = await getLatestCheckpointPath()
  if (checkpointPath == null)
    return null

  const checkpoint = await loadCheckpoint(checkpointPath)
  if (checkpoint == null)
    return null

  note(describeCheckpoint(checkpoint), 'Latest checkpoint found')
  const shouldResume = await prompt<boolean>(confirm({
    initialValue: true,
    message: 'Resume from the latest checkpoint?',
  }))

  if (!shouldResume)
    return null

  return { checkpoint, checkpointPath }
}

const promptForConfig = async (): Promise<BenchmarkRunConfig> => {
  const defaultBaseUrl = (env.PLASTMEM_BASE_URL ?? 'http://localhost:3000').replace(TRAILING_SLASH_RE, '')
  const defaultModel = getRequiredChatModel()
  const allSamples = await loadDefaultSamples()
  const allSampleIds = allSamples.map(sample => sample.sample_id)
  const model = await prompt<string>(text({
    initialValue: defaultModel,
    message: 'Answer model',
    placeholder: defaultModel,
    validate: value => typeof value === 'string' && value.trim().length > 0 ? undefined : 'Model is required',
  }))
  const sampleMode = await prompt<string>(select({
    initialValue: 'custom',
    message: 'Which samples should run?',
    options: [
      { label: 'Minimal subset (42/48)', value: 'minimal' },
      { label: 'Recommended subset (42/44/48/50)', value: 'custom' },
      { label: 'Long-context subset (43/47/48)', value: 'long_context' },
      { label: 'All samples', value: 'all' },
    ],
  }))

  const selectedSampleIds = sampleMode === 'all'
    ? allSampleIds
    : sampleMode === 'minimal'
      ? resolvePresetSampleIds(allSampleIds, MINIMAL_SAMPLE_IDS)
      : sampleMode === 'long_context'
        ? resolvePresetSampleIds(allSampleIds, LONG_CONTEXT_SAMPLE_IDS)
        : await prompt<string[]>(multiselect({
            initialValues: resolvePresetSampleIds(allSampleIds, DEFAULT_SAMPLE_IDS),
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
      { label: 'plast-mem + Full Context', value: 'compare' },
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
    model: model.trim(),
    outFile: timestampedOutputPath(),
    sampleIds: selectedSampleIds.toSorted((left, right) => left.localeCompare(right)),
    useLlmJudge,
    waitForBackground: true,
  }
}

const prepareCheckpoint = async (
  config: BenchmarkRunConfig,
  samples: LoCoMoSample[],
): Promise<{ checkpoint: RunCheckpoint, checkpointPath: string }> => {
  const checkpointPath = buildCheckpointPath(config.outFile)
  return {
    checkpoint: createCheckpoint(config, samples),
    checkpointPath,
  }
}

const main = async (): Promise<void> => {
  try {
    loadEnvFile(resolve(__dirname, '../../../.env'))
  }
  catch { }

  intro('LoCoMo Benchmark')

  const resumed = await loadLatestCheckpoint()
  const config = resumed?.checkpoint.config ?? await promptForConfig()

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

  const { checkpoint, checkpointPath } = resumed ?? await prepareCheckpoint(config, samples)

  note([
    `data: ${config.dataFile}`,
    `out: ${config.outFile}`,
    `checkpoint: ${checkpointPath}`,
    `samples: ${samples.length}`,
    `model: ${config.model}`,
    `baseUrl: ${config.baseUrl}`,
    `llmJudge: ${config.useLlmJudge ? 'on' : 'off'}`,
    `compare: ${config.compareFullContext ? 'plast-mem + Full Context' : 'plast-mem only'}`,
  ].join('\n'), 'Run configuration')

  log.step('Running selected samples')
  const completedCheckpoint = await runBenchmark(checkpoint, checkpointPath, samples)
  log.success('Benchmark run finished')

  printFinalSummary(completedCheckpoint)
  outro(`Results written to ${completedCheckpoint.config.outFile}`)
}

// eslint-disable-next-line @masknet/no-top-level
main().catch((error) => {
  console.error(error)
  exit(1)
})
