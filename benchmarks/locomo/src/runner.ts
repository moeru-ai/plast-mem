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
import { dirname } from 'node:path'

import { progress as createProgress, spinner as createSpinner, log, note } from '@clack/prompts'

import { getVariantOrder, saveCheckpoint } from './checkpoint'
import { runWithConcurrency } from './concurrency'
import { llmJudge, scoreAnswer, scoreAnswerNemoriF1 } from './evaluation'
import { buildFullContext } from './full-context'
import { ingestAll } from './ingest'
import { generateAnswer } from './llm'
import { getContext } from './retrieve'
import {
  computeComparison,
  computeStats,
  renderComparisonMarkdown,
  renderSampleSummaryBlock,
  renderComparison,
  renderStatsMarkdown,
  renderStats,
} from './stats'

const QA_CONCURRENCY = 4

interface EvaluationProgressState {
  advance: (step: number, message: string) => void
  clear: () => void
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
  seed: config.seed,
  timestamp: new Date().toISOString(),
  use_llm_judge: config.useLlmJudge,
})

const getVariantLabel = (variant: BenchmarkVariant): string =>
  variant === 'plastmem' ? 'plast-mem' : 'full-context'

const renderBenchmarkMarkdown = (
  output: BenchmarkOutput,
  completedAt: null | string,
): string => {
  const sections = [`# ${completedAt ?? output.meta.timestamp}`]

  const plastmem = output.variants.plastmem
  if (plastmem != null) {
    sections.push('## plast-mem')
    sections.push(renderStatsMarkdown(plastmem.stats))
  }

  const fullContext = output.variants.full_context
  if (fullContext != null) {
    sections.push('## full-context')
    sections.push(renderStatsMarkdown(fullContext.stats))
  }

  if (output.comparison != null) {
    sections.push('## delta')
    sections.push(renderComparisonMarkdown(output.comparison))
  }

  return `${sections.join('\n\n')}\n`
}

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
  const output = buildBenchmarkOutput(checkpoint)
  const markdownFile = outFile.replace(/\.json$/i, '.md')
  await mkdir(dirname(outFile), { recursive: true })
  await writeFile(outFile, JSON.stringify(output, null, 2))
  await writeFile(markdownFile, renderBenchmarkMarkdown(output, checkpoint.completed_at))
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
  progressState: EvaluationProgressState,
): Promise<PendingQAResult[]> => {
  const qaPairs = sample.qa.filter(qa => qa.category !== 5)
  let retrievedCount = 0
  let answeredCount = 0
  const variantLabel = getVariantLabel(variant)

  const contexts = Array.from<string>({ length: qaPairs.length }).fill('')
  await runWithConcurrency(
    qaPairs.map((qa, index) => async () => {
      contexts[index] = await getContextForVariant(variant, sample, sampleCheckpoint, config, qa.question)
      retrievedCount += 1
      progressState.advance(
        1,
        `Evaluating ${sample.sample_id} ${variantLabel} (${retrievedCount}/${qaPairs.length} retrieved, ${answeredCount}/${qaPairs.length} answered)`,
      )
    }),
    QA_CONCURRENCY,
  )

  const results = Array.from<null | PendingQAResult>({ length: qaPairs.length }).fill(null)
  await runWithConcurrency(
    qaPairs.map((qa, index) => async () => {
      const prediction = await generateAnswer(
        contexts[index] ?? '',
        qa.question,
        qa.category,
        config.model,
        config.seed,
      )
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
      answeredCount += 1
      progressState.advance(
        1,
        `Evaluating ${sample.sample_id} ${variantLabel} (${retrievedCount}/${qaPairs.length} retrieved, ${answeredCount}/${qaPairs.length} answered)`,
      )
    }),
    QA_CONCURRENCY,
  )
  return results.map((result, index) => {
    if (result == null)
      throw new Error(`Missing evaluated result for sample ${sample.sample_id} question #${index + 1}`)
    return result
  })
}

const createEvaluationProgress = (
  sample: LoCoMoSample,
  sampleCheckpoint: SampleCheckpoint,
  compareFullContext: boolean,
): EvaluationProgressState | null => {
  const qaPairs = sample.qa.filter(qa => qa.category !== 5)
  const variantsToEvaluate = getVariantOrder(compareFullContext)
    .filter((variant) => {
      const variantCheckpoint = sampleCheckpoint.variants[variant]
      return variantCheckpoint != null && !variantCheckpoint.eval_done
    })

  const totalSteps = variantsToEvaluate.length * qaPairs.length * 2
  if (totalSteps === 0)
    return null

  const progress = createProgress({ max: totalSteps })
  progress.start(`Evaluating ${sample.sample_id}`)
  return {
    advance: (step, message) => progress.advance(step, message),
    clear: () => progress.clear(),
  }
}

const scoreVariant = async (
  variant: BenchmarkVariant,
  sample: LoCoMoSample,
  config: BenchmarkRunConfig,
  results: PendingQAResult[],
): Promise<QAResult[]> => {
  const spinner = createSpinner()
  spinner.start(`Scoring ${getVariantLabel(variant)} (${results.length} answers)`)

  const scored = Array.from<null | QAResult>({ length: results.length }).fill(null)
  await runWithConcurrency(
    results.map((result, index) => async () => {
      const score = scoreAnswer(result.prediction, result.gold_answer, result.category)
      const nemoriF1Score = scoreAnswerNemoriF1(result.prediction, result.gold_answer)
      const llmScore = config.useLlmJudge
        ? await llmJudge(
            result.prediction,
            result.gold_answer,
            result.question,
            result.category,
            config.model,
            config.seed,
          )
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

  spinner.clear()
  return scored.map((result, index) => {
    if (result == null)
      throw new Error(`Missing scored result for sample ${sample.sample_id} question #${index + 1}`)
    return result
  })
}

const printCompletedSampleSummary = (sample: LoCoMoSample, sampleCheckpoint: SampleCheckpoint): void => {
  const plastmemResults = getScoredResults(sampleCheckpoint.variants.plastmem?.results ?? [])
  const fullContextResults = getScoredResults(sampleCheckpoint.variants.full_context?.results ?? [])

  if (plastmemResults.length > 0 && fullContextResults.length > 0) {
    const plastmemSummary = computeStats(plastmemResults).overall
    const fullContextSummary = computeStats(fullContextResults).overall
    const comparison = computeComparison(plastmemResults, fullContextResults)
    const sampleDelta = comparison.by_sample[sample.sample_id]
    note(
      renderSampleSummaryBlock(plastmemSummary, fullContextSummary, sampleDelta),
      `${sample.sample_id}  n=${plastmemSummary.total}`,
    )
  }
}

const ingestSampleIfNeeded = async (
  sample: LoCoMoSample,
  sampleCheckpoint: SampleCheckpoint,
  config: BenchmarkRunConfig,
  checkpoint: RunCheckpoint,
  checkpointPath: string,
): Promise<void> => {
  if (sampleCheckpoint.ingest_done) {
    log.info(`Reusing ingested sample ${sample.sample_id}`)
    return
  }

  const ids = await ingestAll(
    [sample],
    sampleCheckpoint.conversation_id != null ? { [sample.sample_id]: sampleCheckpoint.conversation_id } : {},
    config.baseUrl,
    1,
    config.waitForBackground,
    async (nextIds) => {
      sampleCheckpoint.conversation_id = nextIds[sample.sample_id] ?? sampleCheckpoint.conversation_id
      await persistState(checkpointPath, checkpoint)
    },
  )

  sampleCheckpoint.conversation_id = ids[sample.sample_id] ?? sampleCheckpoint.conversation_id
  sampleCheckpoint.ingest_done = true
}

const runSample = async (
  sample: LoCoMoSample,
  checkpoint: RunCheckpoint,
  checkpointPath: string,
): Promise<void> => {
  const sampleCheckpoint = checkpoint.samples[sample.sample_id]
  sampleCheckpoint.status = 'running'
  sampleCheckpoint.error = null
  await persistState(checkpointPath, checkpoint)

  try {
    log.step(`Sample ${sample.sample_id}`)
    await ingestSampleIfNeeded(sample, sampleCheckpoint, checkpoint.config, checkpoint, checkpointPath)
    await persistState(checkpointPath, checkpoint)
    const evaluationProgress = createEvaluationProgress(sample, sampleCheckpoint, checkpoint.config.compareFullContext)
    try {
      for (const variant of getVariantOrder(checkpoint.config.compareFullContext)) {
        const variantCheckpoint = sampleCheckpoint.variants[variant]
        if (variantCheckpoint == null)
          continue

        if (!variantCheckpoint.eval_done) {
          variantCheckpoint.results = await evaluateVariant(
            variant,
            sample,
            sampleCheckpoint,
            checkpoint.config,
            evaluationProgress ?? {
              advance: () => {},
              clear: () => {},
            },
          )
          variantCheckpoint.eval_done = true
          await persistState(checkpointPath, checkpoint)
        }

        if (!variantCheckpoint.score_done) {
          variantCheckpoint.results = await scoreVariant(variant, sample, checkpoint.config, variantCheckpoint.results)
          variantCheckpoint.score_done = true
          await persistState(checkpointPath, checkpoint)
        }
      }
    }
    finally {
      evaluationProgress?.clear()
    }

    sampleCheckpoint.status = 'complete'
    await persistState(checkpointPath, checkpoint)
    printCompletedSampleSummary(sample, sampleCheckpoint)
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
  for (const sample of samples) {
    const sampleCheckpoint = checkpoint.samples[sample.sample_id]
    if (sampleCheckpoint?.status === 'complete') {
      log.info(`Sample ${sample.sample_id} already complete, skipping`)
      continue
    }

    await runSample(sample, checkpoint, checkpointPath)
  }

  checkpoint.completed_at = new Date().toISOString()
  await persistState(checkpointPath, checkpoint)
  return checkpoint
}

export const printFinalSummary = (checkpoint: RunCheckpoint): void => {
  const output = buildBenchmarkOutput(checkpoint)
  const plastmem = output.variants.plastmem
  if (plastmem != null)
    note(renderStats(plastmem.stats), 'plast-mem')

  const fullContext = output.variants.full_context
  if (fullContext != null)
    note(renderStats(fullContext.stats), 'full-context')

  if (output.comparison != null)
    note(renderComparison(output.comparison), 'Delta vs Full Context')
}
