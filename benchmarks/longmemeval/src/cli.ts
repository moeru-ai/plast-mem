import type {
  LongMemEvalDataset,
  LongMemEvalOutput,
  LongMemEvalQuestionType,
  LongMemEvalResult,
} from './types'

import process, { env, loadEnvFile } from 'node:process'

import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { setTimeout as sleep } from 'node:timers/promises'
import { fileURLToPath } from 'node:url'

import c from 'tinyrainbow'

import { benchmarkJobStatus } from 'plastmem'

import * as p from '@clack/prompts'

import { name } from '../package.json'
import { judgeAnswer } from './evaluation'
import { ingestAll, loadConversationIds, saveConversationIds } from './ingest'
import { generateSampleAnswer } from './llm'
import { getSampleContext } from './retrieve'
import { computeStats } from './stats'
import { checkDataset, downloadDataset, loadDataset } from './utils/dataset'

const __dirname = dirname(fileURLToPath(import.meta.url))

const INITIAL_WAIT_MS = 10_000
const POLL_INTERVAL_MS = 10_000

type ConversationIdMap = Record<string, string>

interface ConversationStatus {
  apalis_active: number
  done: boolean
  fence_active: boolean
  messages_pending: number
}

const getRequiredEnv = (key: 'OPENAI_CHAT_MODEL' | 'PLASTMEM_BASE_URL'): string => {
  const value = env[key]
  if (value == null || value.length === 0) {
    throw new Error(`Missing required environment variable: ${key}`)
  }
  return value
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
  return res.data as ConversationStatus
}

const waitForConversation = async (
  conversationId: string,
  baseUrl: string,
  spinner: ReturnType<typeof p.spinner>,
): Promise<void> => {
  spinner.start(`Waiting for background jobs ${conversationId.slice(0, 8)}`)
  await sleep(INITIAL_WAIT_MS)

  while (true) {
    const status = await getStatus(baseUrl, conversationId)
    if (status.done) {
      spinner.stop(`Background jobs completed ${conversationId.slice(0, 8)}`)
      return
    }

    spinner.message(
      `Waiting ${conversationId.slice(0, 8)} `
      + `pending=${status.messages_pending} fence=${status.fence_active ? 1 : 0} apalis=${status.apalis_active}`,
    )
    await sleep(POLL_INTERVAL_MS)
  }
}

const buildOutputPath = (): string =>
  resolve(__dirname, `../results/${new Date().toISOString().replace(/[:.]/g, '-')}.json`)

const buildLatestOutputPath = (): string =>
  resolve(__dirname, '../results/latest.json')

const buildConversationIdsPath = (): string =>
  resolve(__dirname, '../results/conversation-ids.json')

const resolveDatasetPath = async (): Promise<string> => {
  const cachedPath = await checkDataset()
  if (cachedPath != null)
    return cachedPath

  const confirmDownload = await p.confirm({
    message: 'The LongMemEval-S dataset was not found. Would you like to download it?',
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

const selectSamples = async (dataset: LongMemEvalDataset): Promise<LongMemEvalDataset> => {
  const selectedQuestionTypes = await promptQuestionTypes(dataset)
  const filteredDataset = dataset.filter(sample => selectedQuestionTypes.includes(sample.question_type))

  if (filteredDataset.length === 0) {
    p.cancel('No samples selected.')
    process.exit(0)
  }

  const sampleLimit = await promptSampleLimit(filteredDataset.length)
  const limitedDataset = filteredDataset.slice(0, sampleLimit)

  p.note([
    `selected question types: ${selectedQuestionTypes.join(', ')}`,
    `filtered samples: ${filteredDataset.length}/${dataset.length}`,
    `selected examples: ${limitedDataset.length}/${filteredDataset.length}`,
    `selected type counts: ${summarizeQuestionTypes(limitedDataset)}`,
  ].join('\n'), 'Run Summary')

  return limitedDataset
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

const loadArtifact = async (path: string): Promise<LongMemEvalOutput | null> => {
  try {
    const content = await readFile(path, 'utf-8')
    return JSON.parse(content) as LongMemEvalOutput
  }
  catch {
    return null
  }
}

const promptReuseArtifact = async (existingCount: number): Promise<boolean> => {
  const reuseExisting = await p.confirm({
    initialValue: true,
    message: `Reuse ${existingCount} completed results from the latest artifact?`,
  })

  if (p.isCancel(reuseExisting)) {
    p.cancel('Operation cancelled.')
    process.exit(0)
  }

  return reuseExisting
}

const promptReuseConversationIds = async (
  reusableCount: number,
  totalCount: number,
): Promise<boolean> => {
  const reuseExisting = await p.confirm({
    initialValue: true,
    message: `Reuse ${reusableCount}/${totalCount} existing conversation ids?`,
  })

  if (p.isCancel(reuseExisting)) {
    p.cancel('Operation cancelled.')
    process.exit(0)
  }

  return reuseExisting
}

const resolveConversationIds = async (
  dataset: LongMemEvalDataset,
  baseUrl: string,
): Promise<ConversationIdMap> => {
  const conversationIdsPath = buildConversationIdsPath()
  const cachedConversationIds = await loadConversationIds(conversationIdsPath)
  const reusableEntries = Object.fromEntries(
    dataset
      .map(sample => [sample.question_id, cachedConversationIds[sample.question_id]])
      .filter(([, conversationId]) => conversationId != null && conversationId.length > 0),
  ) as ConversationIdMap

  let conversationIds: ConversationIdMap = {}
  if (Object.keys(reusableEntries).length > 0) {
    const reuseExisting = await promptReuseConversationIds(
      Object.keys(reusableEntries).length,
      dataset.length,
    )
    if (reuseExisting)
      conversationIds = reusableEntries
  }

  const pendingSamples = dataset.filter((sample) => {
    const conversationId = conversationIds[sample.question_id]
    return conversationId == null || conversationId.length === 0
  })

  if (pendingSamples.length > 0) {
    const ingestSpinner = p.spinner()
    ingestSpinner.start(`Ingesting ${pendingSamples.length} samples`)
    const ingestedConversationIds = await ingestAll(pendingSamples, baseUrl)
    ingestSpinner.stop(`Ingested ${pendingSamples.length} samples`)
    conversationIds = {
      ...conversationIds,
      ...ingestedConversationIds,
    }
    await saveConversationIds(conversationIdsPath, {
      ...cachedConversationIds,
      ...conversationIds,
    })
  }

  return conversationIds
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

const writeArtifact = async (
  outFile: string,
  latestOutFile: string,
  baseUrl: string,
  model: string,
  results: LongMemEvalResult[],
): Promise<void> => {
  const output: LongMemEvalOutput = {
    meta: {
      base_url: baseUrl,
      model,
      timestamp: new Date().toISOString(),
    },
    results,
    stats: computeStats(results),
  }

  await mkdir(dirname(outFile), { recursive: true })
  await writeFile(outFile, JSON.stringify(output, null, 2))
  await writeFile(latestOutFile, JSON.stringify(output, null, 2))
}

const resolveRunState = async (
  dataset: LongMemEvalDataset,
  latestOutFile: string,
): Promise<{
  pendingDataset: LongMemEvalDataset
  results: LongMemEvalResult[]
}> => {
  const artifact = await loadArtifact(latestOutFile)
  if (artifact == null || artifact.results.length === 0) {
    return {
      pendingDataset: dataset,
      results: [],
    }
  }

  const reuseExisting = await promptReuseArtifact(artifact.results.length)
  if (!reuseExisting) {
    return {
      pendingDataset: dataset,
      results: [],
    }
  }

  const completedQuestionIds = new Set(artifact.results.map(result => result.question_id))
  const pendingDataset = dataset.filter(sample => !completedQuestionIds.has(sample.question_id))

  p.note([
    `completed results reused: ${artifact.results.length}`,
    `remaining samples: ${pendingDataset.length}/${dataset.length}`,
  ].join('\n'), 'Resume Summary')

  return {
    pendingDataset,
    results: artifact.results,
  }
}

const main = async () => {
  loadWorkspaceEnv()

  console.clear()
  console.log('')

  p.intro(c.bgCyan(c.black(` ${name} `)))

  const baseUrl = getRequiredEnv('PLASTMEM_BASE_URL')
  const model = getRequiredEnv('OPENAI_CHAT_MODEL')
  const path = await resolveDatasetPath()

  const dataset = await loadDataset(path)

  p.log.info(`file path: ${path}`)
  p.log.info(`loaded samples: ${dataset.length}`)
  p.log.info(`question types: ${summarizeQuestionTypes(dataset)}`)

  const filteredDataset = await selectSamples(dataset)
  const outFile = buildOutputPath()
  const latestOutFile = buildLatestOutputPath()
  const runState = await resolveRunState(filteredDataset, latestOutFile)

  if (runState.pendingDataset.length === 0) {
    p.note([
      formatStatsSummary(runState.results),
      `results file: ${latestOutFile}`,
    ].join('\n'), 'Results')
    p.outro('LongMemEval run complete.')
    return
  }

  logFirstSampleSummary(runState.pendingDataset)

  const results = [...runState.results]
  const conversationIds = await resolveConversationIds(runState.pendingDataset, baseUrl)
  const waitSpinner = p.spinner()
  for (const sample of runState.pendingDataset) {
    const conversationId = conversationIds[sample.question_id]
    if (conversationId == null || conversationId.length === 0) {
      throw new Error(`Missing conversation id for question ${sample.question_id}`)
    }
    await waitForConversation(conversationId, baseUrl, waitSpinner)
  }

  const runSpinner = p.spinner()
  runSpinner.start(`Running retrieval and evaluation for ${runState.pendingDataset.length} samples`)
  for (const [index, sample] of runState.pendingDataset.entries()) {
    const conversationId = conversationIds[sample.question_id]
    if (conversationId == null || conversationId.length === 0) {
      throw new Error(`Missing conversation id for question ${sample.question_id}`)
    }

    runSpinner.message(
      `Evaluating ${index + 1}/${runState.pendingDataset.length} `
      + `${sample.question_id} (${sample.question_type})`,
    )

    const context = await getSampleContext(sample, conversationId, baseUrl)
    const prediction = await generateSampleAnswer(sample, context, model)
    const judged = await judgeAnswer({
      model,
      prediction,
      sample,
    })

    results.push({
      context,
      conversation_id: conversationId,
      gold_answer: String(sample.improved_answer ?? sample.answer),
      prediction,
      question: sample.improved_question ?? sample.question,
      question_id: sample.question_id,
      question_type: sample.question_type,
      score: judged.score,
      verdict: judged.verdict,
    })
    await writeArtifact(outFile, latestOutFile, baseUrl, model, results)
  }
  runSpinner.stop(`Evaluated ${runState.pendingDataset.length} samples`)

  await writeArtifact(outFile, latestOutFile, baseUrl, model, results)

  p.note([
    formatStatsSummary(results),
    `latest results: ${latestOutFile}`,
    `timestamped results: ${outFile}`,
  ].join('\n'), 'Results')

  p.outro('LongMemEval run complete.')
}

// eslint-disable-next-line @masknet/no-top-level
main().catch(console.error)
