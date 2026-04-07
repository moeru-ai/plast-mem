import type {
  BenchmarkRunConfig,
  RunManifest,
  SampleResultFile,
  SampleState,
} from './checkpoint'
import type {
  BenchmarkMeta,
  BenchmarkOutput,
  BenchmarkVariant,
  BenchmarkVariantOutput,
  LoCoMoSample,
  QAPair,
  QAResult,
} from './types'
import type { ConversationStatus } from './wait'

import { mkdir, writeFile } from 'node:fs/promises'
import { dirname } from 'node:path'

import { note } from '@clack/prompts'

import {
  buildOverallJsonPath,
  buildOverallMarkdownPath,
  buildRunManifestPath,
  createEmptySampleResult,
  getVariantOrder,
  loadSampleResult,
  saveRunManifest,
  saveSampleResult,
  saveSampleState,
} from './checkpoint'
import { runWithConcurrency } from './concurrency'
import { BenchmarkDashboard, renderProgressBar } from './dashboard'
import { llmJudge, scoreAnswer, scoreAnswerNemoriF1 } from './evaluation'
import { buildFullContext } from './full-context'
import { ingestAll } from './ingest'
import { generateAnswer } from './llm'
import { getContext } from './retrieve'
import {
  computeComparison,
  computeStats,
  renderComparison,
  renderComparisonMarkdown,
  renderStats,
  renderStatsMarkdown,
} from './stats'

const SAMPLE_VARIANT_CONCURRENCY = 1

interface RunBenchmarkResult {
  manifest: RunManifest
  output: BenchmarkOutput
  sampleResults: Record<string, SampleResultFile>
}

interface SamplePersistence {
  flush: () => Promise<void>
  saveResult: () => Promise<void>
  saveState: () => Promise<void>
  saveStateAndResult: () => Promise<void>
}

const cloneJson = <T>(value: T): T =>
  JSON.parse(JSON.stringify(value)) as T

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

const collectVariantResults = (
  sampleResults: Record<string, SampleResultFile>,
  variant: BenchmarkVariant,
): QAResult[] =>
  Object.values(sampleResults).flatMap(sample => sample.variants[variant]?.results ?? [])

export const buildBenchmarkOutput = (
  config: BenchmarkRunConfig,
  sampleResults: Record<string, SampleResultFile>,
): BenchmarkOutput => {
  const plastmemResults = collectVariantResults(sampleResults, 'plastmem')
  const fullContextResults = collectVariantResults(sampleResults, 'full_context')

  const variants: BenchmarkOutput['variants'] = {
    plastmem: {
      results: plastmemResults,
      stats: computeStats(plastmemResults),
    },
  }

  if (config.compareFullContext) {
    variants.full_context = {
      results: fullContextResults,
      stats: computeStats(fullContextResults),
    }
  }

  return {
    comparison: config.compareFullContext
      ? computeComparison(plastmemResults, fullContextResults)
      : undefined,
    meta: buildMeta(config),
    variants,
  }
}

const writeOverallOutput = async (
  manifest: RunManifest,
  sampleResults: Record<string, SampleResultFile>,
): Promise<BenchmarkOutput> => {
  const outFile = buildOverallJsonPath(manifest.config.outDir)
  const markdownFile = buildOverallMarkdownPath(manifest.config.outDir)
  const output = buildBenchmarkOutput(manifest.config, sampleResults)
  await mkdir(dirname(outFile), { recursive: true })
  await writeFile(outFile, JSON.stringify(output, null, 2))
  await writeFile(markdownFile, renderBenchmarkMarkdown(output, manifest.completed_at))
  return output
}

const createSamplePersistence = (
  outDir: string,
  sampleState: SampleState,
  sampleResult: SampleResultFile,
): SamplePersistence => {
  let writeChain = Promise.resolve()

  const enqueueWrite = async (action: () => Promise<void>): Promise<void> => {
    const nextWrite = writeChain.then(action, action)
    writeChain = nextWrite.catch(() => {})
    return nextWrite
  }

  return {
    flush: async () => {
      await writeChain
    },
    saveResult: async () => {
      const resultSnapshot = cloneJson(sampleResult)
      await enqueueWrite(async () => saveSampleResult(outDir, resultSnapshot))
    },
    saveState: async () => {
      const stateSnapshot = cloneJson(sampleState)
      await enqueueWrite(async () => saveSampleState(outDir, stateSnapshot))
    },
    saveStateAndResult: async () => {
      const stateSnapshot = cloneJson(sampleState)
      const resultSnapshot = cloneJson(sampleResult)
      await enqueueWrite(async () => {
        await saveSampleState(outDir, stateSnapshot)
        await saveSampleResult(outDir, resultSnapshot)
      })
    },
  }
}

const buildSampleVariantOutput = (results: QAResult[]): BenchmarkVariantOutput => ({
  results,
  stats: computeStats(results),
})

const getQaKey = (qa: Pick<QAPair, 'category' | 'question'>): string =>
  `${qa.category}:${qa.question}`

const getResultKey = (result: Pick<QAResult, 'category' | 'question'>): string =>
  `${result.category}:${result.question}`

const normalizeSampleArtifacts = (
  sampleState: SampleState,
  sampleResult: SampleResultFile,
  compareFullContext: boolean,
): void => {
  const activeVariants = new Set(getVariantOrder(compareFullContext))

  sampleState.variants.plastmem ??= { eval_done: false }
  if (compareFullContext)
    sampleState.variants.full_context ??= { eval_done: false }
  else
    delete sampleState.variants.full_context

  for (const variant of getVariantOrder(true)) {
    if (!activeVariants.has(variant)) {
      delete sampleResult.variants[variant]
      continue
    }

    const variantState = sampleState.variants[variant]
    if (variantState == null)
      continue

    if (variantState.eval_done && sampleResult.variants[variant] == null)
      variantState.eval_done = false
  }

  if (sampleState.status === 'running')
    sampleState.status = 'pending'

  const allVariantsComplete = getVariantOrder(compareFullContext).every(variant =>
    sampleState.variants[variant]?.eval_done === true && sampleResult.variants[variant] != null,
  )

  if (sampleState.status === 'complete' && !allVariantsComplete)
    sampleState.status = 'pending'
}

const loadSampleResults = async (
  outDir: string,
  sampleIds: string[],
): Promise<Record<string, SampleResultFile>> => {
  const entries = await Promise.all(sampleIds.map(async (sampleId) => {
    const existing = await loadSampleResult(outDir, sampleId)
    return [sampleId, existing ?? createEmptySampleResult(sampleId)] as const
  }))

  return Object.fromEntries(entries)
}

const formatSampleLine = (
  sampleId: string,
  labelWidth: number,
  detail: string,
): string => `${sampleId.padEnd(labelWidth)}  ${detail}`

const formatPendingLine = (sampleId: string, labelWidth: number): string =>
  formatSampleLine(sampleId, labelWidth, 'pending')

const formatStartingLine = (sampleId: string, labelWidth: number): string =>
  formatSampleLine(sampleId, labelWidth, 'starting')

const formatIngestLine = (
  sampleId: string,
  labelWidth: number,
  done: number,
  total: number,
): string =>
  formatSampleLine(
    sampleId,
    labelWidth,
    `ingest   ${renderProgressBar(done, total)} ${done}/${total}`,
  )

const renderFlag = (value: boolean): string => value ? 'yes' : 'no'

const formatWaitLine = (
  sampleId: string,
  labelWidth: number,
  status: ConversationStatus,
): string =>
  formatSampleLine(
    sampleId,
    labelWidth,
    `wait     pending=${status.messages_pending} fence=${renderFlag(status.fence_active)} seg=${status.segmentation_jobs_active} pc=${status.predict_calibrate_jobs_active} eof=${renderFlag(status.eof_seen)}`,
  )

const formatEvaluateLine = (
  sampleId: string,
  labelWidth: number,
  variant: BenchmarkVariant,
  retrievedCount: number,
  scoredCount: number,
  total: number,
): string =>
  formatSampleLine(
    sampleId,
    labelWidth,
    `${getVariantLabel(variant).padEnd(11)} retrieve ${renderProgressBar(retrievedCount, total)} ${retrievedCount}/${total}  qa ${renderProgressBar(scoredCount, total)} ${scoredCount}/${total}`,
  )

const formatDoneLine = (sampleId: string, labelWidth: number): string =>
  formatSampleLine(sampleId, labelWidth, 'done')

const formatFailedLine = (
  sampleId: string,
  labelWidth: number,
  error: string,
): string => formatSampleLine(sampleId, labelWidth, `failed   ${error}`)

const getContextForVariant = async (
  variant: BenchmarkVariant,
  sample: LoCoMoSample,
  sampleState: SampleState,
  config: BenchmarkRunConfig,
  question: string,
  onRetry?: (message: string) => void,
): Promise<string> => {
  if (variant === 'plastmem') {
    const conversationId = sampleState.conversation_id
    if (conversationId == null || conversationId.length === 0)
      throw new Error(`Missing conversation_id for sample ${sample.sample_id}`)
    return getContext(conversationId, question, config.baseUrl, onRetry)
  }

  return buildFullContext(sample, question)
}

const evaluateVariant = async (
  variant: BenchmarkVariant,
  sample: LoCoMoSample,
  sampleState: SampleState,
  sampleResult: SampleResultFile,
  config: BenchmarkRunConfig,
  persistence: SamplePersistence,
  dashboard: BenchmarkDashboard,
  labelWidth: number,
): Promise<void> => {
  const qaPairs = sample.qa.filter(qa => qa.category !== 5)
  const existingResults = sampleResult.variants[variant]?.results ?? []
  const resultsByKey = new Map(existingResults.map(result => [getResultKey(result), result]))
  let retrievedCount = resultsByKey.size
  let scoredCount = resultsByKey.size

  const updateEvaluateLine = (): void => {
    dashboard.setLine(
      sample.sample_id,
      formatEvaluateLine(
        sample.sample_id,
        labelWidth,
        variant,
        retrievedCount,
        scoredCount,
        qaPairs.length,
      ),
    )
  }

  const orderedResults = (): QAResult[] =>
    qaPairs
      .map((qa) => {
        const result = resultsByKey.get(getQaKey(qa))
        return result ?? null
      })
      .filter((result): result is QAResult => result != null)

  const pendingQaPairs = qaPairs.filter(qa => !resultsByKey.has(getQaKey(qa)))
  if (pendingQaPairs.length === 0) {
    sampleResult.variants[variant] = buildSampleVariantOutput(existingResults)
    sampleState.variants[variant] ??= { eval_done: false }
    sampleState.variants[variant].eval_done = true
    updateEvaluateLine()
    await persistence.saveStateAndResult()
    return
  }

  updateEvaluateLine()

  await runWithConcurrency(
    pendingQaPairs.map(qa => async () => {
      const context = await getContextForVariant(
        variant,
        sample,
        sampleState,
        config,
        qa.question,
        () => {},
      )
      retrievedCount += 1
      updateEvaluateLine()

      const prediction = await generateAnswer(
        context,
        qa.question,
        qa.category,
        config.model,
        config.seed,
        () => {},
      )
      const score = scoreAnswer(prediction, qa.answer, qa.category)
      const nemoriF1Score = scoreAnswerNemoriF1(prediction, qa.answer)
      const llmScore = config.useLlmJudge
        ? await llmJudge(
            prediction,
            qa.answer,
            qa.question,
            qa.category,
            config.model,
            config.seed,
          )
        : 0

      resultsByKey.set(getQaKey(qa), {
        category: qa.category,
        context_retrieved: context,
        evidence: qa.evidence,
        gold_answer: qa.answer,
        llm_judge_score: llmScore,
        nemori_f1_score: nemoriF1Score,
        prediction,
        question: qa.question,
        sample_id: sample.sample_id,
        score,
      })

      const nextResults = orderedResults()
      sampleResult.variants[variant] = buildSampleVariantOutput(nextResults)
      await persistence.saveResult()

      scoredCount += 1
      updateEvaluateLine()
    }),
    SAMPLE_VARIANT_CONCURRENCY,
  )

  sampleResult.variants[variant] = buildSampleVariantOutput(orderedResults())
  sampleState.variants[variant] ??= { eval_done: false }
  sampleState.variants[variant].eval_done = true
  await persistence.saveStateAndResult()
}

const ingestSampleIfNeeded = async (
  sample: LoCoMoSample,
  sampleState: SampleState,
  config: BenchmarkRunConfig,
  persistence: SamplePersistence,
  dashboard: BenchmarkDashboard,
  labelWidth: number,
): Promise<void> => {
  if (sampleState.ingest_done)
    return

  const ids = await ingestAll(
    [sample],
    sampleState.conversation_id != null ? { [sample.sample_id]: sampleState.conversation_id } : {},
    config.baseUrl,
    1,
    config.waitForBackground,
    {
      onConversationAssigned: async (_sampleId, conversationId) => {
        sampleState.conversation_id = conversationId
        await persistence.saveState()
      },
      onEvent: () => {},
      onIngestProgress: (sampleId, _conversationId, done, total) => {
        dashboard.setLine(sampleId, formatIngestLine(sampleId, labelWidth, done, total))
      },
      onWaitStatus: (sampleId, _conversationId, status) => {
        dashboard.setLine(sampleId, formatWaitLine(sampleId, labelWidth, status))
      },
    },
  )

  sampleState.conversation_id = ids[sample.sample_id] ?? sampleState.conversation_id
  sampleState.ingest_done = true
  await persistence.saveState()
}

const runSample = async (
  sample: LoCoMoSample,
  manifest: RunManifest,
  sampleState: SampleState,
  sampleResult: SampleResultFile,
  dashboard: BenchmarkDashboard,
  labelWidth: number,
): Promise<void> => {
  normalizeSampleArtifacts(sampleState, sampleResult, manifest.config.compareFullContext)

  const activeVariants = getVariantOrder(manifest.config.compareFullContext)
  const alreadyComplete = sampleState.status === 'complete'
    && activeVariants.every(variant =>
      sampleState.variants[variant]?.eval_done === true && sampleResult.variants[variant] != null,
    )

  if (alreadyComplete) {
    dashboard.setLine(sample.sample_id, formatDoneLine(sample.sample_id, labelWidth))
    return
  }

  const persistence = createSamplePersistence(manifest.config.outDir, sampleState, sampleResult)
  sampleState.status = 'running'
  sampleState.error = null
  await persistence.saveState()
  dashboard.setLine(sample.sample_id, formatStartingLine(sample.sample_id, labelWidth))

  try {
    await ingestSampleIfNeeded(sample, sampleState, manifest.config, persistence, dashboard, labelWidth)

    for (const variant of activeVariants) {
      const variantState = sampleState.variants[variant]
      if (variantState == null)
        continue

      if (variantState.eval_done && sampleResult.variants[variant] != null)
        continue

      await evaluateVariant(
        variant,
        sample,
        sampleState,
        sampleResult,
        manifest.config,
        persistence,
        dashboard,
        labelWidth,
      )
    }

    sampleState.status = 'complete'
    await persistence.saveState()
    dashboard.setLine(sample.sample_id, formatDoneLine(sample.sample_id, labelWidth))
  }
  catch (error) {
    sampleState.error = error instanceof Error ? error.message : String(error)
    sampleState.status = 'failed'
    await persistence.saveState()
    dashboard.setLine(sample.sample_id, formatFailedLine(sample.sample_id, labelWidth, sampleState.error))
  }
  finally {
    await persistence.flush()
  }
}

export const runBenchmark = async (
  manifest: RunManifest,
  sampleStates: Record<string, SampleState>,
  samples: LoCoMoSample[],
): Promise<RunBenchmarkResult> => {
  const sampleResults = await loadSampleResults(manifest.config.outDir, samples.map(sample => sample.sample_id))
  const labelWidth = Math.max(...samples.map(sample => sample.sample_id.length))
  const dashboard = new BenchmarkDashboard(samples.map(sample => sample.sample_id))

  for (const sample of samples) {
    const sampleState = sampleStates[sample.sample_id]
    const sampleResult = sampleResults[sample.sample_id]
    if (sampleState == null)
      throw new Error(`Missing sample state for ${sample.sample_id}`)
    if (sampleResult == null)
      throw new Error(`Missing sample result for ${sample.sample_id}`)
    normalizeSampleArtifacts(sampleState, sampleResult, manifest.config.compareFullContext)
    dashboard.setLine(sample.sample_id, formatPendingLine(sample.sample_id, labelWidth))
  }
  await dashboard.flush()

  try {
    await runWithConcurrency(
      samples.map(sample => async () => {
        const sampleState = sampleStates[sample.sample_id]
        const sampleResult = sampleResults[sample.sample_id]
        if (sampleState == null || sampleResult == null)
          return
        await runSample(sample, manifest, sampleState, sampleResult, dashboard, labelWidth)
      }),
      manifest.config.sampleConcurrency,
    )
  }
  finally {
    await dashboard.stop()
  }

  manifest.completed_at = new Date().toISOString()
  await saveRunManifest(buildRunManifestPath(manifest.config.outDir), manifest)
  const output = await writeOverallOutput(manifest, sampleResults)

  return {
    manifest,
    output,
    sampleResults,
  }
}

export const printFinalSummary = (output: BenchmarkOutput): void => {
  const plastmem = output.variants.plastmem
  if (plastmem != null)
    note(renderStats(plastmem.stats), 'plast-mem')

  const fullContext = output.variants.full_context
  if (fullContext != null)
    note(renderStats(fullContext.stats), 'full-context')

  if (output.comparison != null)
    note(renderComparison(output.comparison), 'Delta vs Full Context')
}
