import type {
  LongMemEvalDataset,
  LongMemEvalOutput,
  LongMemEvalQuestionType,
  LongMemEvalResult,
} from './types'

import process, { env, loadEnvFile } from 'node:process'

import { mkdir, writeFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { setTimeout as sleep } from 'node:timers/promises'
import { fileURLToPath } from 'node:url'

import c from 'tinyrainbow'

import { benchmarkJobStatus } from 'plastmem'

import * as p from '@clack/prompts'

import { name } from '../package.json'
import { judgeAnswer } from './evaluation'
import { ingestAll } from './ingest'
import { generateSampleAnswer } from './llm'
import { getSampleContext } from './retrieve'
import { computeStats } from './stats'
import { checkDataset, downloadDataset, loadDataset } from './utils/dataset'

const __dirname = dirname(fileURLToPath(import.meta.url))

const INITIAL_WAIT_MS = 10_000
const POLL_INTERVAL_MS = 10_000

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

  p.note([
    `selected question types: ${selectedQuestionTypes.join(', ')}`,
    `filtered samples: ${filteredDataset.length}/${dataset.length}`,
    `filtered type counts: ${summarizeQuestionTypes(filteredDataset)}`,
  ].join('\n'), 'Run Summary')

  return filteredDataset
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

const evaluateSamples = async (
  dataset: LongMemEvalDataset,
  baseUrl: string,
  model: string,
): Promise<LongMemEvalResult[]> => {
  const ingestSpinner = p.spinner()
  ingestSpinner.start(`Ingesting ${dataset.length} samples`)
  const conversationIds = await ingestAll(dataset, baseUrl)
  ingestSpinner.stop(`Ingested ${dataset.length} samples`)

  const waitSpinner = p.spinner()
  for (const sample of dataset) {
    const conversationId = conversationIds[sample.question_id]
    if (conversationId == null || conversationId.length === 0) {
      throw new Error(`Missing conversation id for question ${sample.question_id}`)
    }
    await waitForConversation(conversationId, baseUrl, waitSpinner)
  }

  const runSpinner = p.spinner()
  runSpinner.start(`Running retrieval and evaluation for ${dataset.length} samples`)
  const results: LongMemEvalResult[] = []

  for (const [index, sample] of dataset.entries()) {
    const conversationId = conversationIds[sample.question_id]
    if (conversationId == null || conversationId.length === 0) {
      throw new Error(`Missing conversation id for question ${sample.question_id}`)
    }

    runSpinner.message(
      `Evaluating ${index + 1}/${dataset.length} `
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
      gold_answer: sample.improved_answer ?? sample.answer,
      prediction,
      question: sample.improved_question ?? sample.question,
      question_id: sample.question_id,
      question_type: sample.question_type,
      score: judged.score,
      verdict: judged.verdict,
    })
  }

  runSpinner.stop(`Evaluated ${dataset.length} samples`)
  return results
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
  logFirstSampleSummary(filteredDataset)

  const results = await evaluateSamples(filteredDataset, baseUrl, model)

  const outFile = buildOutputPath()
  await writeArtifact(outFile, baseUrl, model, results)

  p.note([
    formatStatsSummary(results),
    `results file: ${outFile}`,
  ].join('\n'), 'Results')

  p.outro('LongMemEval run complete.')
}

// eslint-disable-next-line @masknet/no-top-level
main().catch(console.error)
