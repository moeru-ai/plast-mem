import type {
  BenchmarkMeta,
  BenchmarkOutput,
  BenchmarkRunConfig,
  BenchmarkVariant,
  LoCoMoSample,
  PendingQAResult,
  QAResult,
  RunCheckpoint,
  SampleCheckpoint,
} from './types'

import { mkdir, readFile, writeFile } from 'node:fs/promises'
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
  getVariantOrder,
  isCheckpointCompatible,
  loadCheckpoint,
  resetCheckpointFile,
  saveCheckpoint,
} from './checkpoint'
import { llmJudge, scoreAnswer, scoreAnswerNemoriF1 } from './evaluation'
import { buildFullContext } from './full-context'
import { ingestAll, loadConversationIds, saveConversationIds } from './ingest'
import { generateAnswer } from './llm'
import { getContext } from './retrieve'
import { computeComparison, computeStats, printComparison, printStats } from './stats'
import { waitForAll } from './wait'

const __dirname = dirname(fileURLToPath(import.meta.url))

const DEFAULT_CONCURRENCY = 4
const IDS_FILE = resolve(__dirname, '../data/conversation_ids.json')
const COLON_DOT_RE = /[:.]/g
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

const runWithConcurrency = async (
  tasks: Array<() => Promise<void>>,
  concurrency: number,
): Promise<void> => {
  if (tasks.length === 0)
    return

  const limit = Math.max(1, Math.floor(concurrency))
  let nextIndex = 0

  const worker = async (): Promise<void> => {
    while (true) {
      const currentIndex = nextIndex
      nextIndex += 1
      if (currentIndex >= tasks.length)
        return
      await tasks[currentIndex]()
    }
  }

  await Promise.all(Array.from({ length: Math.min(limit, tasks.length) }).fill(0).map(async () => worker()))
}

const isScoredResult = (result: PendingQAResult): result is QAResult =>
  result.llm_judge_score != null
  && result.nemori_f1_score != null
  && result.score != null

const getScoredResults = (results: PendingQAResult[]): QAResult[] =>
  results.filter(isScoredResult)

const buildMeta = (config: BenchmarkRunConfig): BenchmarkMeta => ({
  base_url: config.baseUrl,
  compare_full_context: config.compareFullContext,
  data_file: config.dataFile,
  model: config.model,
  sample_ids: config.sampleIds,
  timestamp: new Date().toISOString(),
  use_llm_judge: config.useLlmJudge,
})

const buildOutputFromCheckpoint = (checkpoint: RunCheckpoint): BenchmarkOutput => {
  const plastmemResults = Object.values(checkpoint.samples)
    .flatMap(sample => getScoredResults(sample.variants.plastmem?.results ?? []))

  const fullContextResults = Object.values(checkpoint.samples)
    .flatMap(sample => getScoredResults(sample.variants.full_context?.results ?? []))

  const variants: BenchmarkOutput['variants'] = {
    plastmem: {
      results: plastmemResults,
      stats: computeStats(plastmemResults),
    },
  }

  if (checkpoint.config.compareFullContext) {
    variants.full_context = {
      results: fullContextResults,
      stats: computeStats(fullContextResults),
    }
  }

  return {
    comparison: checkpoint.config.compareFullContext
      ? computeComparison(plastmemResults, fullContextResults)
      : undefined,
    meta: buildMeta(checkpoint.config),
    variants,
  }
}

const writeOutput = async (
  outFile: string,
  checkpoint: RunCheckpoint,
): Promise<void> => {
  await mkdir(dirname(outFile), { recursive: true })
  await writeFile(outFile, JSON.stringify(buildOutputFromCheckpoint(checkpoint), null, 2))
}

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
    initialValue: 'all',
    message: 'Which samples should run?',
    options: [
      { label: 'All samples', value: 'all' },
      { label: 'Select individual samples', value: 'custom' },
    ],
  }))

  const selectedSampleIds = sampleMode === 'all'
    ? allSamples.map(sample => sample.sample_id)
    : await prompt<string[]>(multiselect({
        initialValues: allSamples.map(sample => sample.sample_id),
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

const persistState = async (
  checkpointPath: string,
  checkpoint: RunCheckpoint,
): Promise<void> => {
  await saveCheckpoint(checkpointPath, checkpoint)
  await writeOutput(checkpoint.config.outFile, checkpoint)
}

const getContextForVariant = async (
  variant: BenchmarkVariant,
  sample: LoCoMoSample,
  sampleCheckpoint: SampleCheckpoint,
  config: BenchmarkRunConfig,
  question: string,
): Promise<string> => {
  if (variant === 'plastmem') {
    const conversationId = sampleCheckpoint.conversation_id
    if (conversationId == null || conversationId.length === 0)
      throw new Error(`Missing conversation_id for sample ${sample.sample_id}`)
    return getContext(conversationId, question, config.baseUrl)
  }

  return buildFullContext(sample, config, question)
}

const evaluateVariant = async (
  variant: BenchmarkVariant,
  sample: LoCoMoSample,
  sampleCheckpoint: SampleCheckpoint,
  config: BenchmarkRunConfig,
): Promise<PendingQAResult[]> => {
  const qaPairs = sample.qa.filter(qa => qa.category !== 5)
  const label = variant === 'plastmem' ? 'plast-mem' : 'Full Context'
  console.log(`  ${label}: evaluating ${qaPairs.length} questions`)

  const contexts = Array.from<string>({ length: qaPairs.length }).fill('')
  await runWithConcurrency(
    qaPairs.map((qa, index) => async () => {
      contexts[index] = await getContextForVariant(variant, sample, sampleCheckpoint, config, qa.question)
    }),
    config.concurrency,
  )

  const results = Array.from<null | PendingQAResult>({ length: qaPairs.length }).fill(null)
  await runWithConcurrency(
    qaPairs.map((qa, index) => async () => {
      const prediction = await generateAnswer(contexts[index] ?? '', qa.question, qa.category, config.model)
      results[index] = {
        category: qa.category,
        context_retrieved: contexts[index] ?? '',
        evidence: qa.evidence,
        gold_answer: qa.answer,
        llm_judge_score: null,
        nemori_f1_score: null,
        prediction,
        question: qa.question,
        sample_id: sample.sample_id,
        score: null,
      }
      console.log(`    [${index + 1}/${qaPairs.length}] answered`)
    }),
    config.concurrency,
  )

  return results.map((result, index) => {
    if (result == null)
      throw new Error(`Missing evaluated result for sample ${sample.sample_id} question #${index + 1}`)
    return result
  })
}

const scoreVariant = async (
  variant: BenchmarkVariant,
  sample: LoCoMoSample,
  config: BenchmarkRunConfig,
  results: PendingQAResult[],
): Promise<QAResult[]> => {
  const label = variant === 'plastmem' ? 'plast-mem' : 'Full Context'
  console.log(`  ${label}: scoring ${results.length} answers`)

  const scored = Array.from<null | QAResult>({ length: results.length }).fill(null)
  await runWithConcurrency(
    results.map((result, index) => async () => {
      const score = scoreAnswer(result.prediction, result.gold_answer, result.category)
      const nemoriF1Score = scoreAnswerNemoriF1(result.prediction, result.gold_answer)
      const llmScore = config.useLlmJudge
        ? await llmJudge(result.prediction, result.gold_answer, result.question, result.category, config.model)
        : 0

      scored[index] = {
        ...result,
        llm_judge_score: llmScore,
        nemori_f1_score: nemoriF1Score,
        score,
      }
      console.log(
        `    [${index + 1}/${results.length}] `
        + `f1=${score.toFixed(2)} nemoriF1=${nemoriF1Score.toFixed(2)} llm=${llmScore.toFixed(2)}`,
      )
    }),
    config.concurrency,
  )

  console.log(`  ${label}: sample ${sample.sample_id} score complete`)
  return scored.map((result, index) => {
    if (result == null)
      throw new Error(`Missing scored result for sample ${sample.sample_id} question #${index + 1}`)
    return result
  })
}

const ingestSampleIfNeeded = async (
  sample: LoCoMoSample,
  sampleCheckpoint: SampleCheckpoint,
  config: BenchmarkRunConfig,
  conversationIds: Record<string, string>,
): Promise<void> => {
  if (sampleCheckpoint.ingest_done) {
    console.log(`  Reusing ingested sample ${sample.sample_id}`)
    return
  }

  const ids = await ingestAll(
    [sample],
    {
      ...conversationIds,
      ...(sampleCheckpoint.conversation_id != null ? { [sample.sample_id]: sampleCheckpoint.conversation_id } : {}),
    },
    config.baseUrl,
    1,
    config.waitForBackground,
    async (nextIds) => {
      Object.assign(conversationIds, nextIds)
      await saveConversationIds(IDS_FILE, nextIds)
    },
  )

  Object.assign(conversationIds, ids)
  sampleCheckpoint.conversation_id = ids[sample.sample_id] ?? sampleCheckpoint.conversation_id
  sampleCheckpoint.ingest_done = true

  if (config.waitForBackground) {
    const conversationId = sampleCheckpoint.conversation_id
    if (conversationId == null || conversationId.length === 0)
      throw new Error(`No conversation_id after ingest for sample ${sample.sample_id}`)
    await waitForAll([conversationId], config.baseUrl)
  }
}

const processSample = async (
  sample: LoCoMoSample,
  checkpoint: RunCheckpoint,
  checkpointPath: string,
  conversationIds: Record<string, string>,
): Promise<void> => {
  const sampleCheckpoint = checkpoint.samples[sample.sample_id]
  sampleCheckpoint.status = 'running'
  sampleCheckpoint.error = null
  await persistState(checkpointPath, checkpoint)

  try {
    console.log(`\n── Sample ${sample.sample_id} ──`)
    await ingestSampleIfNeeded(sample, sampleCheckpoint, checkpoint.config, conversationIds)
    await persistState(checkpointPath, checkpoint)

    for (const variant of getVariantOrder(checkpoint.config.compareFullContext)) {
      const variantCheckpoint = sampleCheckpoint.variants[variant]
      if (variantCheckpoint == null)
        continue

      if (!variantCheckpoint.eval_done) {
        variantCheckpoint.results = await evaluateVariant(variant, sample, sampleCheckpoint, checkpoint.config)
        variantCheckpoint.eval_done = true
        await persistState(checkpointPath, checkpoint)
      }

      if (!variantCheckpoint.score_done) {
        variantCheckpoint.results = await scoreVariant(variant, sample, checkpoint.config, variantCheckpoint.results)
        variantCheckpoint.score_done = true
        await persistState(checkpointPath, checkpoint)
      }
    }

    sampleCheckpoint.status = 'complete'
    await persistState(checkpointPath, checkpoint)
  }
  catch (error) {
    sampleCheckpoint.error = error instanceof Error ? error.message : String(error)
    sampleCheckpoint.status = 'failed'
    await persistState(checkpointPath, checkpoint)
    console.error(`  Sample ${sample.sample_id} failed: ${sampleCheckpoint.error}`)
  }
}

const printFinalSummary = (checkpoint: RunCheckpoint): void => {
  const output = buildOutputFromCheckpoint(checkpoint)
  const plastmem = output.variants.plastmem
  if (plastmem != null) {
    console.log('\nplast-mem')
    printStats(plastmem.stats)
  }

  const fullContext = output.variants.full_context
  if (fullContext != null) {
    console.log('Full Context')
    printStats(fullContext.stats)
  }

  if (output.comparison != null)
    printComparison(output.comparison)
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
  const conversationIds = await loadConversationIds(IDS_FILE)
  for (const sample of Object.values(checkpoint.samples)) {
    if (sample.conversation_id != null && sample.conversation_id.length > 0)
      conversationIds[sample.sample_id] = sample.conversation_id
  }

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

  for (const sample of samples) {
    const sampleCheckpoint = checkpoint.samples[sample.sample_id]
    if (sampleCheckpoint?.status === 'complete') {
      console.log(`\n── Sample ${sample.sample_id} already complete, skipping ──`)
      continue
    }

    await processSample(sample, checkpoint, checkpointPath, conversationIds)
  }

  checkpoint.completed_at = new Date().toISOString()
  await persistState(checkpointPath, checkpoint)
  progress.stop('Benchmark run finished')

  printFinalSummary(checkpoint)
  outro(`Results written to ${config.outFile}`)
}

// eslint-disable-next-line @masknet/no-top-level
main().catch((error) => {
  console.error(error)
  exit(1)
})
