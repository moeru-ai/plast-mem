import type { BenchmarkRunConfig, RunCheckpoint } from './checkpoint'
import type { LoCoMoSample } from './types'

import { readFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { env, exit, loadEnvFile } from 'node:process'
import { fileURLToPath } from 'node:url'

import {
  cancel,
  confirm,
  intro,
  isCancel,
  multiselect,
  note,
  outro,
  select,
  spinner,
  text,
} from '@clack/prompts'

import {
  buildCheckpointPath,
  createCheckpoint,
  isCheckpointCompatible,
  loadCheckpoint,
  resetCheckpointFile,
} from './checkpoint'
import { printFinalSummary, runBenchmark } from './runner'

const __dirname = dirname(fileURLToPath(import.meta.url))

const DEFAULT_CONCURRENCY = 4
const COLON_DOT_RE = /[:.]/g
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
  resolve(__dirname, `../results/${new Date().toISOString().replace(COLON_DOT_RE, '-')}.json`)

const loadSamples = async (dataFile: string): Promise<LoCoMoSample[]> => {
  const raw = await readFile(dataFile, 'utf-8')
  return JSON.parse(raw) as LoCoMoSample[]
}

const promptForConfig = async (): Promise<BenchmarkRunConfig> => {
  const defaultDataFile = resolve(__dirname, '../data/locomo10.json')
  const defaultOutFile = timestampedOutputPath()
  const defaultBaseUrl = (env.PLASTMEM_BASE_URL ?? 'http://localhost:3000').replace(TRAILING_SLASH_RE, '')
  const defaultModel = env.OPENAI_CHAT_MODEL ?? 'gpt-4o-mini'

  const dataFile = resolve(await prompt<string>(text({
    defaultValue: defaultDataFile,
    message: 'LoCoMo dataset path',
    placeholder: defaultDataFile,
  })))

  const allSamples = await loadSamples(dataFile)
  const sampleMode = await prompt<string>(select({
    initialValue: 'custom',
    message: 'Which samples should run?',
    options: [
      { label: 'Recommended subset (42/44/48/50)', value: 'custom' },
      { label: 'All samples', value: 'all' },
    ],
  }))

  const defaultSelectedSampleIds = allSamples
    .map(sample => sample.sample_id)
    .filter(sampleId => DEFAULT_SAMPLE_IDS.includes(sampleId as typeof DEFAULT_SAMPLE_IDS[number]))

  const selectedSampleIds = sampleMode === 'all'
    ? allSamples.map(sample => sample.sample_id)
    : await prompt<string[]>(multiselect({
        initialValues: defaultSelectedSampleIds,
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

  const concurrencyRaw = await prompt<string>(text({
    defaultValue: String(DEFAULT_CONCURRENCY),
    message: 'QA concurrency',
    placeholder: String(DEFAULT_CONCURRENCY),
    validate: (value: string | undefined) => {
      if (value == null)
        return 'Enter a positive integer.'
      const parsed = Number.parseInt(value, 10)
      return Number.isFinite(parsed) && parsed > 0 ? undefined : 'Enter a positive integer.'
    },
  }))

  const waitForBackground = await prompt<boolean>(confirm({
    initialValue: true,
    message: 'Wait for background jobs after each sample ingest?',
  }))

  const useLlmJudge = await prompt<boolean>(confirm({
    initialValue: false,
    message: 'Enable LLM judge scoring?',
  }))

  const baseUrl = (await prompt<string>(text({
    defaultValue: defaultBaseUrl,
    message: 'plast-mem base URL',
    placeholder: defaultBaseUrl,
  }))).replace(TRAILING_SLASH_RE, '')

  const model = await prompt<string>(text({
    defaultValue: defaultModel,
    message: 'Answer model',
    placeholder: defaultModel,
  }))

  const outFile = resolve(await prompt<string>(text({
    defaultValue: defaultOutFile,
    message: 'Result output file',
    placeholder: defaultOutFile,
  })))

  return {
    baseUrl,
    compareFullContext: compareMode === 'compare',
    concurrency: Number.parseInt(concurrencyRaw, 10),
    dataFile,
    model,
    outFile,
    sampleIds: selectedSampleIds.toSorted((left, right) => left.localeCompare(right)),
    useLlmJudge,
    waitForBackground,
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

const prepareCheckpoint = async (
  config: BenchmarkRunConfig,
  samples: LoCoMoSample[],
): Promise<{ checkpoint: RunCheckpoint, checkpointPath: string }> => {
  const checkpointPath = buildCheckpointPath(config.outFile)
  const existing = await loadCheckpoint(checkpointPath)

  if (existing != null && isCheckpointCompatible(existing, config)) {
    note(describeCheckpoint(existing), 'Existing checkpoint found')
    const shouldResume = await prompt<boolean>(confirm({
      initialValue: true,
      message: 'Resume from the existing checkpoint?',
    }))

    if (shouldResume)
      return { checkpoint: existing, checkpointPath }
  }

  await resetCheckpointFile(checkpointPath)
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

  const config = await promptForConfig()

  if (config.useLlmJudge && (env.OPENAI_API_KEY == null || env.OPENAI_API_KEY.length === 0)) {
    cancel('OPENAI_API_KEY not set for LLM judge mode.')
    exit(1)
  }

  const allSamples = await loadSamples(config.dataFile)
  const samples = allSamples.filter(sample => config.sampleIds.includes(sample.sample_id))
  if (samples.length === 0) {
    cancel('No samples selected.')
    exit(1)
  }

  const { checkpoint, checkpointPath } = await prepareCheckpoint(config, samples)

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

  const progress = spinner()
  progress.start('Running selected samples')
  const completedCheckpoint = await runBenchmark(checkpoint, checkpointPath, samples)
  progress.stop('Benchmark run finished')

  printFinalSummary(completedCheckpoint)
  outro(`Results written to ${config.outFile}`)
}

// eslint-disable-next-line @masknet/no-top-level
main().catch((error) => {
  console.error(error)
  exit(1)
})
