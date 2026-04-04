import type { BenchmarkRunConfig, RunCheckpoint } from './checkpoint'
import type {
  LongMemEvalDataset,
  LongMemEvalOutput,
  LongMemEvalOutputItem,
  LongMemEvalQuestionType,
  LongMemEvalResult,
  LongMemEvalSample,
} from './types'

import process, { env, loadEnvFile } from 'node:process'

import { mkdir, readdir, writeFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { setTimeout as sleep } from 'node:timers/promises'
import { fileURLToPath } from 'node:url'

import c from 'tinyrainbow'

import { uuid } from '@insel-null/uuid'
import { benchmarkJobStatus } from 'plastmem'

import * as p from '@clack/prompts'

import { name } from '../package.json'
import {
  buildCheckpointFingerprint,
  buildCheckpointPath,
  collectResults,
  createCheckpoint,
  loadCheckpoint,
  saveCheckpoint,
} from './checkpoint'
import { runWithConcurrency } from './concurrency'
import { judgeAnswer } from './evaluation'
import { countSampleMessages, ingestSample, ingestSampleWithProgress } from './ingest'
import { generateSampleAnswer } from './llm'
import { getSampleContext } from './retrieve'
import { computeStats } from './stats'
import {
  checkDataset,
  DATASET_FILE_ID,
  downloadDataset,
  loadDataset,
} from './utils/dataset'

const __dirname = dirname(fileURLToPath(import.meta.url))

const INITIAL_WAIT_MS = 1_000
const POLL_INTERVAL_MS = 5_000
const CHECKPOINT_FILE_SUFFIX = '.checkpoint.json'
const COLON_DOT_RE = /[:.]/g
const INGEST_CONCURRENCY = 2

interface ConversationStatus {
  admissible_for_add: boolean
  done: boolean
  fence_active: boolean
  flushable: boolean
  messages_pending: number
  predict_calibrate_jobs_active: number
  segmentation_jobs_active: number
}

interface PreparedRunState {
  checkpoint: RunCheckpoint
  checkpointPath: string
  config: BenchmarkRunConfig
  dataset: LongMemEvalDataset
  datasetById: Map<string, LongMemEvalSample>
  datasetPath: string
  latestOutFile: string
  pendingDataset: LongMemEvalDataset
  resumed: null | {
    checkpoint: RunCheckpoint
    checkpointPath: string
  }
}

const getRequiredEnv = (key: 'OPENAI_CHAT_MODEL' | 'PLASTMEM_BASE_URL'): string => {
  const value = env[key]
  if (value == null || value.length === 0) {
    throw new Error(`Missing required environment variable: ${key}`)
  }
  return value
}

const getOptionalChatSeed = (): number | undefined => {
  const rawSeed = env.OPENAI_CHAT_SEED?.trim()
  if (rawSeed == null || rawSeed.length === 0)
    return undefined

  if (!/^-?\d+$/.test(rawSeed))
    return undefined

  return Number.parseInt(rawSeed, 10)
}

const loadWorkspaceEnv = (): void => {
  try {
    loadEnvFile(resolve(__dirname, '../../../.env'))
  }
  catch {}
}

const buildQuestionTypeCounts = (dataset: LongMemEvalDataset): Record<LongMemEvalQuestionType, number> => {
  const counts: Record<LongMemEvalQuestionType, number> = {
    'knowledge-update': 0,
    'multi-session': 0,
    'single-session-assistant': 0,
    'single-session-preference': 0,
    'single-session-user': 0,
    'temporal-reasoning': 0,
  }

  for (const sample of dataset)
    counts[sample.question_type] += 1

  return counts
}

const summarizeQuestionTypes = (dataset: LongMemEvalDataset): string =>
  Object.entries(buildQuestionTypeCounts(dataset))
    .filter(([, count]) => count > 0)
    .map(([type, count]) => `${type}=${count}`)
    .join(', ')

const promptQuestionTypes = async (dataset: LongMemEvalDataset): Promise<LongMemEvalQuestionType[]> => {
  const counts = buildQuestionTypeCounts(dataset)
  const selected = await p.multiselect({
    initialValues: Object.entries(counts)
      .filter(([, count]) => count > 0)
      .map(([type]) => type),
    message: 'Choose question types to run',
    options: Object.entries(counts)
      .filter(([, count]) => count > 0)
      .map(([type, count]) => ({
        hint: `${count} samples`,
        label: type,
        value: type,
      })),
    required: false,
  })

  if (p.isCancel(selected)) {
    p.cancel('Operation cancelled.')
    process.exit(0)
  }

  return selected as LongMemEvalQuestionType[]
}

const promptSampleLimit = async (availableCount: number): Promise<number> => {
  const selected = await p.select({
    initialValue: '5',
    message: 'How many examples should this run use?',
    options: [
      { hint: 'fastest smoke test', label: '1', value: '1' },
      { hint: 'recommended quick check', label: '5', value: '5' },
      { hint: 'small batch', label: '10', value: '10' },
      { hint: 'larger sample', label: '20', value: '20' },
      { hint: `${availableCount} available`, label: 'all', value: 'all' },
    ],
  })

  if (p.isCancel(selected)) {
    p.cancel('Operation cancelled.')
    process.exit(0)
  }

  if (selected === 'all')
    return availableCount

  const parsed = Number.parseInt(selected, 10)
  return Math.min(parsed, availableCount)
}

const getStatus = async (
  baseUrl: string,
  conversationId: string,
): Promise<ConversationStatus> => {
  const res = await benchmarkJobStatus({
    baseUrl,
    query: { conversation_id: conversationId },
    throwOnError: true,
  })
  return {
    admissible_for_add: res.data.admissible_for_add,
    done: res.data.done,
    fence_active: res.data.fence_active,
    flushable: res.data.flushable,
    messages_pending: res.data.messages_pending,
    predict_calibrate_jobs_active: res.data.predict_calibrate_jobs_active,
    segmentation_jobs_active: res.data.segmentation_jobs_active,
  }
}

const waitForConversation = async (
  conversationId: string,
  baseUrl: string,
  onStatus: (message: string) => void,
): Promise<void> => {
  onStatus(`Waiting for background jobs ${conversationId.slice(0, 8)}`)
  await sleep(INITIAL_WAIT_MS)

  while (true) {
    const status = await getStatus(baseUrl, conversationId)
    if (status.done)
      return

    onStatus(
      `Waiting ${conversationId.slice(0, 8)} `
      + `pending=${status.messages_pending} fence=${status.fence_active ? 1 : 0} `
      + `segmentation=${status.segmentation_jobs_active} predict_calibrate=${status.predict_calibrate_jobs_active}`,
    )
    await sleep(POLL_INTERVAL_MS)
  }
}

const buildOutputPath = (): string =>
  resolve(__dirname, `../results/${new Date().toISOString().replace(COLON_DOT_RE, '-')}.json`)

const buildLatestOutputPath = (): string =>
  resolve(__dirname, '../results/latest.json')

const resolveDatasetPath = async (): Promise<string> => {
  const cachedPath = await checkDataset()
  if (cachedPath != null)
    return cachedPath

  const confirmDownload = await p.confirm({
    message: 'The LongMemEval-S cleaned dataset was not found. Would you like to download it?',
  })

  if (confirmDownload !== true) {
    p.cancel('Operation cancelled.')
    process.exit(0)
  }

  const spinner = p.spinner()
  spinner.start('Downloading via huggingface...')
  try {
    const downloadedPath = await downloadDataset()
    spinner.stop('Downloaded')
    return downloadedPath
  }
  catch (err) {
    spinner.error(err instanceof Error ? err.message : undefined)
    p.cancel('Operation cancelled.')
    process.exit(0)
  }
}

const selectSamples = async (dataset: LongMemEvalDataset): Promise<{
  selectedDataset: LongMemEvalDataset
  selectedQuestionTypes: LongMemEvalQuestionType[]
}> => {
  const selectedQuestionTypes = await promptQuestionTypes(dataset)
  const filteredDataset = dataset.filter(sample => selectedQuestionTypes.includes(sample.question_type))

  if (filteredDataset.length === 0) {
    p.cancel('No samples selected.')
    process.exit(0)
  }

  const sampleLimit = await promptSampleLimit(filteredDataset.length)
  const limitedDataset = selectedQuestionTypes.flatMap((questionType) => {
    const typeSamples = filteredDataset.filter(sample => sample.question_type === questionType)
    return typeSamples.slice(0, sampleLimit)
  })

  p.note([
    `selected question types: ${selectedQuestionTypes.join(', ')}`,
    `filtered samples: ${filteredDataset.length}/${dataset.length}`,
    `selected examples per type: ${sampleLimit === filteredDataset.length ? 'all' : sampleLimit}`,
    `selected examples total: ${limitedDataset.length}`,
    `selected type counts: ${summarizeQuestionTypes(limitedDataset)}`,
  ].join('\n'), 'Run Summary')

  return {
    selectedDataset: limitedDataset,
    selectedQuestionTypes,
  }
}

const logFirstSampleSummary = (dataset: LongMemEvalDataset): void => {
  const firstSample = dataset[0]
  const sessionCount = firstSample.haystack_sessions.length
  const turnCount = firstSample.haystack_sessions.reduce((total, session) => total + session.length, 0)
  p.log.info(`first sample: ${firstSample.question_id} (${firstSample.question_type})`)
  p.log.info(`first sample sessions: ${sessionCount}`)
  p.log.info(`first sample turns: ${turnCount}`)
  p.log.info(`first sample answer sessions: ${firstSample.answer_session_ids.length}`)
  p.log.info(`first question: ${firstSample.question}`)
}

const loadDatasetForPath = async (path: string): Promise<LongMemEvalDataset> => loadDataset(path)

const getLatestCheckpointPath = async (): Promise<null | string> => {
  try {
    const resultsDir = resolve(__dirname, '../results')
    const entries = await readdir(resultsDir, { withFileTypes: true })
    const checkpointNames = entries
      .filter(entry => entry.isFile() && entry.name.endsWith(CHECKPOINT_FILE_SUFFIX))
      .map(entry => entry.name)
      .toSorted((left, right) => right.localeCompare(left))

    return checkpointNames[0] == null ? null : resolve(resultsDir, checkpointNames[0])
  }
  catch {
    return null
  }
}

const describeCheckpoint = (checkpointPath: string, checkpoint: RunCheckpoint): string => {
  const samples = Object.values(checkpoint.samples)
  const complete = samples.filter(sample => sample.status === 'complete').length
  const failed = samples.filter(sample => sample.status === 'failed').length
  const running = samples.filter(sample => sample.status === 'running').length
  const pending = samples.length - complete - failed - running

  return [
    `checkpoint: ${checkpointPath}`,
    `started: ${checkpoint.started_at}`,
    `updated: ${checkpoint.updated_at}`,
    `complete: ${complete}`,
    `failed: ${failed}`,
    `running: ${running}`,
    `pending: ${pending}`,
  ].join('\n')
}

const loadLatestResumeCheckpoint = async () => {
  const checkpointPath = await getLatestCheckpointPath()
  if (checkpointPath == null)
    return null

  const checkpoint = await loadCheckpoint(checkpointPath)
  if (checkpoint == null || checkpoint.completed_at != null)
    return null

  p.note(describeCheckpoint(checkpointPath, checkpoint), 'Latest checkpoint found')
  const shouldResume = await p.confirm({
    initialValue: true,
    message: 'Resume from the latest checkpoint?',
  })

  if (shouldResume !== true) {
    if (!p.isCancel(shouldResume))
      return null
    p.cancel('Operation cancelled.')
    process.exit(0)
  }

  return { checkpoint, checkpointPath }
}

const loadDatasetForCheckpoint = async (config: BenchmarkRunConfig): Promise<LongMemEvalDataset> => {
  try {
    return await loadDatasetForPath(config.dataFile)
  }
  catch {
    const fallbackPath = await resolveDatasetPath()
    return loadDatasetForPath(fallbackPath)
  }
}

const formatStatsSummary = (results: LongMemEvalResult[]): string => {
  const stats = computeStats(results)
  const byQuestionType = Object.entries(stats.by_question_type)
    .filter(([type]) => stats.by_question_type_count[type as LongMemEvalQuestionType] > 0)
    .map(([type, score]) => `${type}=${(score * 100).toFixed(2)}%`)
    .join(', ')

  return [
    `overall: ${(stats.overall * 100).toFixed(2)}%`,
    `total: ${stats.total}`,
    `by question type: ${byQuestionType}`,
  ].join('\n')
}

const summarizeFailures = (checkpoint: RunCheckpoint): string[] =>
  Object.values(checkpoint.samples)
    .filter(sample => sample.status === 'failed')
    .map(sample => `${sample.question_id}: ${sample.error ?? 'Unknown error'}`)

const buildOutputItems = (results: LongMemEvalResult[], datasetById: Map<string, LongMemEvalSample>): LongMemEvalOutputItem[] =>
  results.map((result) => {
    const sample = datasetById.get(result.question_id)
    const answer = sample == null ? result.gold_answer : String(sample.answer)

    return {
      item_id: result.question_id,
      metrics: {
        accuracy: result.score,
        detailed_results: [{
          ...result,
          answer,
          is_correct: result.score === 1,
          is_invalid: false,
          question_date: sample?.question_date ?? '',
          response: result.prediction,
        }],
        is_correct: result.score === 1,
        is_invalid: false,
      },
    }
  })

const writeArtifact = async (
  checkpointPath: string,
  latestOutFile: string,
  checkpoint: Parameters<typeof collectResults>[0],
  datasetById: Map<string, LongMemEvalSample>,
): Promise<void> => {
  const results = collectResults(checkpoint)
  const output: LongMemEvalOutput = {
    item_results: buildOutputItems(results, datasetById),
    meta: {
      base_url: checkpoint.config.baseUrl,
      checkpoint_path: checkpointPath,
      dataset: checkpoint.config.datasetName,
      model: checkpoint.config.model,
      seed: checkpoint.config.seed,
      timestamp: new Date().toISOString(),
    },
    stats: computeStats(results),
  }

  await mkdir(dirname(checkpoint.config.outFile), { recursive: true })
  await writeFile(checkpoint.config.outFile, JSON.stringify(output, null, 2))
  await writeFile(latestOutFile, JSON.stringify(output, null, 2))
}

const persistState = async (
  checkpointPath: string,
  latestOutFile: string,
  checkpoint: Parameters<typeof collectResults>[0],
  datasetById: Map<string, LongMemEvalSample>,
): Promise<void> => {
  await saveCheckpoint(checkpointPath, checkpoint)
  await writeArtifact(checkpointPath, latestOutFile, checkpoint, datasetById)
}

const createRunConfig = (
  baseUrl: string,
  model: string,
  seed: number | undefined,
  dataFile: string,
  selectedQuestionTypes: LongMemEvalQuestionType[],
  dataset: LongMemEvalDataset,
): BenchmarkRunConfig => ({
  baseUrl,
  dataFile,
  datasetName: DATASET_FILE_ID,
  model,
  outFile: buildOutputPath(),
  questionTypes: [...selectedQuestionTypes],
  sampleIds: dataset.map(sample => sample.question_id),
  seed,
  waitForBackground: true,
})

const createRunResult = (
  sample: LongMemEvalSample,
  conversationId: string,
  context: string,
  prediction: string,
  score: 0 | 1,
  verdict: string,
): LongMemEvalResult => ({
  context,
  conversation_id: conversationId,
  gold_answer: String(sample.answer),
  prediction,
  question: sample.question,
  question_id: sample.question_id,
  question_type: sample.question_type,
  score,
  verdict,
})

const resolveFreshRun = async (): Promise<{
  checkpoint: RunCheckpoint
  checkpointPath: string
  config: BenchmarkRunConfig
  dataset: LongMemEvalDataset
  datasetPath: string
}> => {
  const baseUrl = getRequiredEnv('PLASTMEM_BASE_URL')
  const model = getRequiredEnv('OPENAI_CHAT_MODEL')
  const seed = getOptionalChatSeed()
  const datasetPath = await resolveDatasetPath()
  const allDataset = await loadDatasetForPath(datasetPath)

  p.log.info(`file path: ${datasetPath}`)
  p.log.info(`loaded samples: ${allDataset.length}`)
  p.log.info(`question types: ${summarizeQuestionTypes(allDataset)}`)

  const selected = await selectSamples(allDataset)
  const dataset = selected.selectedDataset
  const config = createRunConfig(baseUrl, model, seed, datasetPath, selected.selectedQuestionTypes, dataset)
  const checkpoint = createCheckpoint(config, dataset)
  const checkpointPath = buildCheckpointPath(config.outFile)

  return {
    checkpoint,
    checkpointPath,
    config,
    dataset,
    datasetPath,
  }
}

const prepareRunState = async (): Promise<PreparedRunState> => {
  const resumed = await loadLatestResumeCheckpoint()
  const latestOutFile = buildLatestOutputPath()

  let checkpoint: RunCheckpoint
  let checkpointPath: string
  let config: BenchmarkRunConfig
  let dataset: LongMemEvalDataset
  let datasetPath: string

  if (resumed != null) {
    checkpoint = resumed.checkpoint
    checkpointPath = resumed.checkpointPath
    config = checkpoint.config
    dataset = await loadDatasetForCheckpoint(config)
    datasetPath = config.dataFile
  }
  else {
    const fresh = await resolveFreshRun()
    checkpoint = fresh.checkpoint
    checkpointPath = fresh.checkpointPath
    config = fresh.config
    dataset = fresh.dataset
    datasetPath = fresh.datasetPath
  }

  const selectedIdSet = new Set(config.sampleIds)
  const orderedDataset = dataset
    .filter(sample => selectedIdSet.has(sample.question_id))
    .toSorted((left, right) => config.sampleIds.indexOf(left.question_id) - config.sampleIds.indexOf(right.question_id))

  if (buildCheckpointFingerprint(config) !== checkpoint.fingerprint)
    throw new Error('Checkpoint fingerprint mismatch.')

  const datasetById = new Map(orderedDataset.map(sample => [sample.question_id, sample]))
  const pendingDataset = orderedDataset.filter((sample) => {
    const sampleCheckpoint = checkpoint.samples[sample.question_id]
    return sampleCheckpoint == null || sampleCheckpoint.status !== 'complete'
  })

  return {
    checkpoint,
    checkpointPath,
    config,
    dataset: orderedDataset,
    datasetById,
    datasetPath,
    latestOutFile,
    pendingDataset,
    resumed,
  }
}

const printRunConfig = (state: PreparedRunState): void => {
  p.note([
    `dataset: ${state.config.datasetName}`,
    `file path: ${state.datasetPath}`,
    `selected samples: ${state.dataset.length}`,
    `question types: ${state.config.questionTypes.join(', ')}`,
    `seed: ${state.config.seed ?? 'unset'}`,
    `checkpoint: ${state.checkpointPath}`,
    `output: ${state.config.outFile}`,
  ].join('\n'), state.resumed == null ? 'Run Config' : 'Resumed Config')
}

const ingestPendingSamples = async (state: PreparedRunState): Promise<void> => {
  const samplesToIngest = state.pendingDataset.filter((sample) => {
    const sampleCheckpoint = state.checkpoint.samples[sample.question_id]
    return sampleCheckpoint != null && !sampleCheckpoint.ingest_done && sampleCheckpoint.status !== 'failed'
  })
  if (samplesToIngest.length === 0)
    return

  const ingestProgress = p.progress({ max: Math.max(samplesToIngest.length, 1) })
  let completed = 0
  let failed = 0
  let persistChain = Promise.resolve()

  const persistSequentially = async (): Promise<void> => {
    persistChain = persistChain.then(async () => {
      await persistState(state.checkpointPath, state.latestOutFile, state.checkpoint, state.datasetById)
    })
    await persistChain
  }

  ingestProgress.start(
    `Pre-ingesting ${samplesToIngest.length} samples `
    + `(concurrency ${INGEST_CONCURRENCY}, completed ${completed}/${samplesToIngest.length})`,
  )

  const tasks = samplesToIngest.map(sample => async () => {
    const sampleCheckpoint = state.checkpoint.samples[sample.question_id]
    if (sampleCheckpoint == null)
      throw new Error(`Missing checkpoint entry for ${sample.question_id}`)

    let conversationId = sampleCheckpoint.conversation_id
    if (conversationId == null || conversationId.length === 0) {
      conversationId = uuid.v7()
      sampleCheckpoint.conversation_id = conversationId
      await persistSequentially()
    }

    try {
      await ingestSample(sample, conversationId, state.config.baseUrl)
      sampleCheckpoint.ingest_done = true
      sampleCheckpoint.error = null
      completed += 1
      ingestProgress.advance(
        1,
        `Pre-ingesting ${samplesToIngest.length} samples `
        + `(concurrency ${INGEST_CONCURRENCY}, completed ${completed}/${samplesToIngest.length}, failed ${failed})`,
      )
      await persistSequentially()
      p.log.info(`Ingested ${sample.question_id} (${countSampleMessages(sample)} messages)`)
    }
    catch (error) {
      sampleCheckpoint.error = error instanceof Error ? error.message : String(error)
      sampleCheckpoint.status = 'failed'
      failed += 1
      ingestProgress.advance(
        1,
        `Pre-ingesting ${samplesToIngest.length} samples `
        + `(concurrency ${INGEST_CONCURRENCY}, completed ${completed}/${samplesToIngest.length}, failed ${failed})`,
      )
      await persistSequentially()
      p.log.error(`Sample ${sample.question_id} failed during ingest: ${sampleCheckpoint.error}`)
    }
  })

  await runWithConcurrency(tasks, INGEST_CONCURRENCY)
  ingestProgress.stop(
    `Pre-ingested ${samplesToIngest.length} samples `
    + `(completed ${completed}, failed ${failed})`,
  )
}

const finalizeCompletedRun = async (state: PreparedRunState): Promise<void> => {
  const results = collectResults(state.checkpoint)
  await writeArtifact(state.checkpointPath, state.latestOutFile, state.checkpoint, state.datasetById)
  p.note([
    formatStatsSummary(results),
    `results file: ${state.config.outFile}`,
  ].join('\n'), 'Results')
  p.outro('LongMemEval run complete.')
}

const runPendingSamples = async (state: PreparedRunState): Promise<void> => {
  await ingestPendingSamples(state)

  const evaluableDataset = state.pendingDataset.filter((sample) => {
    const sampleCheckpoint = state.checkpoint.samples[sample.question_id]
    return sampleCheckpoint != null && sampleCheckpoint.status !== 'failed'
  })

  if (evaluableDataset.length === 0)
    return

  logFirstSampleSummary(evaluableDataset)
  const evaluationProgress = p.progress({ max: Math.max(evaluableDataset.length, 1) })
  evaluationProgress.start(`Evaluating ${evaluableDataset.length} samples 0/${evaluableDataset.length}`)
  let finished = 0

  for (const [index, sample] of evaluableDataset.entries()) {
    const sampleCheckpoint = state.checkpoint.samples[sample.question_id]
    if (sampleCheckpoint == null)
      throw new Error(`Missing checkpoint entry for ${sample.question_id}`)

    const samplePrefix = `[${index + 1}/${evaluableDataset.length}] ${sample.question_id} (${sample.question_type})`
    const setStage = (stage: string) => {
      p.log.info(`${samplePrefix} ${stage}`)
    }

    sampleCheckpoint.status = 'running'
    sampleCheckpoint.error = null
    await persistState(state.checkpointPath, state.latestOutFile, state.checkpoint, state.datasetById)

    try {
      let conversationId = sampleCheckpoint.conversation_id
      if (conversationId == null || conversationId.length === 0) {
        conversationId = uuid.v7()
        sampleCheckpoint.conversation_id = conversationId
      }

      if (!sampleCheckpoint.ingest_done) {
        setStage('Ingesting')
        await ingestSampleWithProgress(sample, conversationId, state.config.baseUrl)
        sampleCheckpoint.ingest_done = true
        await persistState(state.checkpointPath, state.latestOutFile, state.checkpoint, state.datasetById)
      }

      if (state.config.waitForBackground)
        await waitForConversation(conversationId, state.config.baseUrl, setStage)

      setStage('Retrieving')
      const context = await getSampleContext(sample, conversationId, state.config.baseUrl)

      setStage('Answering')
      const prediction = await generateSampleAnswer(sample, context, state.config.model, state.config.seed)

      setStage('Judging')
      const judged = await judgeAnswer({
        model: state.config.model,
        prediction,
        sample,
        seed: state.config.seed,
      })

      sampleCheckpoint.result = createRunResult(
        sample,
        conversationId,
        context,
        prediction,
        judged.score,
        judged.verdict,
      )
      sampleCheckpoint.status = 'complete'
      await persistState(state.checkpointPath, state.latestOutFile, state.checkpoint, state.datasetById)
      finished += 1
      evaluationProgress.advance(1, `Evaluating ${evaluableDataset.length} samples ${finished}/${evaluableDataset.length}`)
    }
    catch (error) {
      sampleCheckpoint.error = error instanceof Error ? error.message : String(error)
      sampleCheckpoint.status = 'failed'
      await persistState(state.checkpointPath, state.latestOutFile, state.checkpoint, state.datasetById)
      p.log.error(`Sample ${sample.question_id} failed: ${sampleCheckpoint.error}`)
      finished += 1
      evaluationProgress.advance(1, `Evaluating ${evaluableDataset.length} samples ${finished}/${evaluableDataset.length}`)
    }
  }

  state.checkpoint.completed_at = new Date().toISOString()
  await persistState(state.checkpointPath, state.latestOutFile, state.checkpoint, state.datasetById)
  evaluationProgress.stop(`Evaluated ${evaluableDataset.length} samples`)
}

const main = async () => {
  loadWorkspaceEnv()

  console.clear()
  console.log('')

  p.intro(c.bgCyan(c.black(` ${name} `)))

  const state = await prepareRunState()
  printRunConfig(state)

  if (state.pendingDataset.length === 0)
    return finalizeCompletedRun(state)

  await runPendingSamples(state)

  const results = collectResults(state.checkpoint)
  const failures = summarizeFailures(state.checkpoint)
  p.note([
    formatStatsSummary(results),
    `failed: ${failures.length}`,
    `latest results: ${state.latestOutFile}`,
    `timestamped results: ${state.config.outFile}`,
    `checkpoint: ${state.checkpointPath}`,
  ].join('\n'), 'Results')

  if (failures.length > 0)
    p.log.error(`Failed samples:\n${failures.join('\n')}`)

  p.outro('LongMemEval run complete.')
}

// eslint-disable-next-line @masknet/no-top-level
main().catch(console.error)
