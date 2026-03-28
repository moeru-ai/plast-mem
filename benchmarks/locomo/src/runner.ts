import type {
  BenchmarkRunConfig,
  RunCheckpoint,
  SampleCheckpoint,
} from './checkpoint'
import type {
  BenchmarkMeta,
  BenchmarkOutput,
  BenchmarkVariant,
  LoCoMoSample,
  PendingQAResult,
  QAResult,
} from './types'

import { mkdir, writeFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { log } from '@clack/prompts'

import { getVariantOrder, saveCheckpoint } from './checkpoint'
import { llmJudge, scoreAnswer, scoreAnswerNemoriF1 } from './evaluation'
import { buildFullContext } from './full-context'
import { ingestAll, loadConversationIds, saveConversationIds } from './ingest'
import { generateAnswer } from './llm'
import { getContext } from './retrieve'
import { computeComparison, computeStats, printComparison, printStats } from './stats'
import { waitForAll } from './wait'

const __dirname = dirname(fileURLToPath(import.meta.url))
const IDS_FILE = resolve(__dirname, '../data/conversation_ids.json')
const QA_CONCURRENCY = 4

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

export const buildBenchmarkOutput = (checkpoint: RunCheckpoint): BenchmarkOutput => {
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
  await writeFile(outFile, JSON.stringify(buildBenchmarkOutput(checkpoint), null, 2))
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

  return buildFullContext(sample, question)
}

const evaluateVariant = async (
  variant: BenchmarkVariant,
  sample: LoCoMoSample,
  sampleCheckpoint: SampleCheckpoint,
  config: BenchmarkRunConfig,
): Promise<PendingQAResult[]> => {
  const qaPairs = sample.qa.filter(qa => qa.category !== 5)
  const label = variant === 'plastmem' ? 'plast-mem' : 'Full Context'
  log.message(`${label}: evaluating ${qaPairs.length} questions`)

  const contexts = Array.from<string>({ length: qaPairs.length }).fill('')
  await runWithConcurrency(
    qaPairs.map((qa, index) => async () => {
      contexts[index] = await getContextForVariant(variant, sample, sampleCheckpoint, config, qa.question)
    }),
    QA_CONCURRENCY,
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
    }),
    QA_CONCURRENCY,
  )

  log.success(`${label}: evaluation complete for ${qaPairs.length} questions`)
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
  log.message(`${label}: scoring ${results.length} answers`)

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
    }),
    QA_CONCURRENCY,
  )

  const completed = scored.filter((result): result is QAResult => result != null)
  const avgScore = completed.length > 0
    ? completed.reduce((sum, result) => sum + result.score, 0) / completed.length
    : 0
  const avgNemoriF1 = completed.length > 0
    ? completed.reduce((sum, result) => sum + result.nemori_f1_score, 0) / completed.length
    : 0
  const avgLlm = completed.length > 0
    ? completed.reduce((sum, result) => sum + result.llm_judge_score, 0) / completed.length
    : 0

  log.success(
    `${label}: sample ${sample.sample_id} score complete `
    + `f1=${avgScore.toFixed(2)} nemoriF1=${avgNemoriF1.toFixed(2)} llm=${avgLlm.toFixed(2)}`,
  )
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
    log.info(`Reusing ingested sample ${sample.sample_id}`)
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

const runSample = async (
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
    log.step(`Sample ${sample.sample_id}`)
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
    log.error(`Sample ${sample.sample_id} failed: ${sampleCheckpoint.error}`)
  }
}

export const runBenchmark = async (
  checkpoint: RunCheckpoint,
  checkpointPath: string,
  samples: LoCoMoSample[],
): Promise<RunCheckpoint> => {
  const conversationIds = await loadConversationIds(IDS_FILE)
  for (const sample of Object.values(checkpoint.samples)) {
    if (sample.conversation_id != null && sample.conversation_id.length > 0)
      conversationIds[sample.sample_id] = sample.conversation_id
  }

  for (const sample of samples) {
    const sampleCheckpoint = checkpoint.samples[sample.sample_id]
    if (sampleCheckpoint?.status === 'complete') {
      log.info(`Sample ${sample.sample_id} already complete, skipping`)
      continue
    }

    await runSample(sample, checkpoint, checkpointPath, conversationIds)
  }

  checkpoint.completed_at = new Date().toISOString()
  await persistState(checkpointPath, checkpoint)
  return checkpoint
}

export const printFinalSummary = (checkpoint: RunCheckpoint): void => {
  const output = buildBenchmarkOutput(checkpoint)
  const plastmem = output.variants.plastmem
  if (plastmem != null) {
    log.step('plast-mem')
    printStats(plastmem.stats)
  }

  const fullContext = output.variants.full_context
  if (fullContext != null) {
    log.step('Full Context')
    printStats(fullContext.stats)
  }

  if (output.comparison != null)
    printComparison(output.comparison)
}
