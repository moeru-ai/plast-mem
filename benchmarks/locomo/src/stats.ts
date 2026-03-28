import type {
  BenchmarkComparisonMetric,
  BenchmarkComparisonSummary,
  BenchmarkScoreSummary,
  BenchmarkStats,
  QACategory,
  QAResult,
} from './types'

import { log } from '@clack/prompts'

const CATEGORIES: QACategory[] = [1, 2, 3, 4, 5]
const CATEGORY_NAMES: Record<QACategory, string> = {
  1: 'multi-hop',
  2: 'temporal',
  3: 'open-domain',
  4: 'single-hop',
  5: 'adversarial',
}

const avg = (scores: number[]): number =>
  scores.length > 0 ? scores.reduce((a, b) => a + b, 0) / scores.length : 0

const computeScoreSummary = (results: QAResult[]): BenchmarkScoreSummary => {
  const byCategory = Object.fromEntries(
    CATEGORIES.map(c => [c, [] as number[]]),
  ) as Record<QACategory, number[]>

  const byCategoryLlm = Object.fromEntries(
    CATEGORIES.map(c => [c, [] as number[]]),
  ) as Record<QACategory, number[]>

  const byCategoryNemoriF1 = Object.fromEntries(
    CATEGORIES.map(c => [c, [] as number[]]),
  ) as Record<QACategory, number[]>

  for (const r of results) {
    byCategory[r.category].push(r.score)
    byCategoryLlm[r.category].push(r.llm_judge_score)
    byCategoryNemoriF1[r.category].push(r.nemori_f1_score)
  }

  return {
    by_category: Object.fromEntries(CATEGORIES.map(c => [c, avg(byCategory[c])])) as Record<QACategory, number>,
    by_category_count: Object.fromEntries(CATEGORIES.map(c => [c, byCategory[c].length])) as Record<QACategory, number>,
    by_category_llm: Object.fromEntries(CATEGORIES.map(c => [c, avg(byCategoryLlm[c])])) as Record<QACategory, number>,
    by_category_nemori_f1: Object.fromEntries(CATEGORIES.map(c => [c, avg(byCategoryNemoriF1[c])])) as Record<QACategory, number>,
    overall: avg(results.map(r => r.score)),
    overall_llm: avg(results.map(r => r.llm_judge_score)),
    overall_nemori_f1: avg(results.map(r => r.nemori_f1_score)),
    total: results.length,
  }
}

export const computeStats = (results: QAResult[]): BenchmarkStats => {
  const resultsBySample = new Map<string, QAResult[]>()

  for (const result of results) {
    const sampleResults = resultsBySample.get(result.sample_id)
    if (sampleResults == null)
      resultsBySample.set(result.sample_id, [result])
    else
      sampleResults.push(result)
  }

  const bySample = Object.fromEntries(
    [...resultsBySample.entries()]
      .toSorted(([sampleA], [sampleB]) => sampleA.localeCompare(sampleB))
      .map(([sampleId, sampleResults]) => [sampleId, computeScoreSummary(sampleResults)]),
  ) as Record<string, BenchmarkScoreSummary>

  return {
    by_sample: bySample,
    overall: computeScoreSummary(results),
  }
}

const subtractMetric = (
  plastmem: BenchmarkScoreSummary,
  fullContext: BenchmarkScoreSummary,
): BenchmarkComparisonMetric => ({
  llm_judge_delta: plastmem.overall_llm - fullContext.overall_llm,
  nemori_f1_delta: plastmem.overall_nemori_f1 - fullContext.overall_nemori_f1,
  score_delta: plastmem.overall - fullContext.overall,
})

export const computeComparison = (
  plastmemResults: QAResult[],
  fullContextResults: QAResult[],
): BenchmarkComparisonSummary => {
  const plastmemStats = computeStats(plastmemResults)
  const fullContextStats = computeStats(fullContextResults)

  return {
    by_category: Object.fromEntries(
      CATEGORIES.map(category => [category, {
        llm_judge_delta:
          plastmemStats.overall.by_category_llm[category] - fullContextStats.overall.by_category_llm[category],
        nemori_f1_delta:
          plastmemStats.overall.by_category_nemori_f1[category] - fullContextStats.overall.by_category_nemori_f1[category],
        score_delta:
          plastmemStats.overall.by_category[category] - fullContextStats.overall.by_category[category],
      }]),
    ) as Record<QACategory, BenchmarkComparisonMetric>,
    by_sample: Object.fromEntries(
      [...new Set([
        ...Object.keys(plastmemStats.by_sample),
        ...Object.keys(fullContextStats.by_sample),
      ])].toSorted((left, right) => left.localeCompare(right)).map((sampleId) => {
        const plastmemSummary = plastmemStats.by_sample[sampleId] ?? computeScoreSummary([])
        const fullContextSummary = fullContextStats.by_sample[sampleId] ?? computeScoreSummary([])
        return [sampleId, subtractMetric(plastmemSummary, fullContextSummary)]
      }),
    ) as Record<string, BenchmarkComparisonMetric>,
    full_context: fullContextStats.overall,
    overall: subtractMetric(plastmemStats.overall, fullContextStats.overall),
    plastmem: plastmemStats.overall,
  }
}

const formatMetric = (label: string, value: number): string =>
  `${label} ${(value * 100).toFixed(2)}%`

const formatSummaryLine = (summary: BenchmarkScoreSummary): string =>
  `${formatMetric('F1', summary.overall)}  `
  + `${formatMetric('NemoriF1', summary.overall_nemori_f1)}  `
  + `${formatMetric('LLM', summary.overall_llm)}  `
  + `n=${summary.total}`

export const printSampleSummary = (
  label: string,
  sampleId: string,
  summary: BenchmarkScoreSummary,
): void => {
  log.message(`${label} ${sampleId}  ${formatSummaryLine(summary)}`)
}

export const printSampleComparison = (
  sampleId: string,
  metric: BenchmarkComparisonMetric,
): void => {
  log.message([
    'delta ',
    sampleId,
    '  ',
    formatMetric('F1', metric.score_delta),
    '  ',
    formatMetric('NemoriF1', metric.nemori_f1_delta),
    '  ',
    formatMetric('LLM', metric.llm_judge_delta),
  ].join(''))
}

export const printStats = (stats: BenchmarkStats): void => {
  log.message(`overall  ${formatSummaryLine(stats.overall)}`)

  const sampleIds = Object.keys(stats.by_sample)
  if (sampleIds.length > 0)
    log.message('samples')

  for (const sampleId of sampleIds) {
    log.message(`  ${sampleId}  ${formatSummaryLine(stats.by_sample[sampleId])}`)
  }

  log.message('categories')
  for (const category of CATEGORIES) {
    const count = stats.overall.by_category_count[category]
    if (count === 0)
      continue

    log.message(
      `  c${category} ${CATEGORY_NAMES[category].padEnd(12)}  `
      + `${formatMetric('F1', stats.overall.by_category[category])}  `
      + `${formatMetric('NemoriF1', stats.overall.by_category_nemori_f1[category])}  `
      + `${formatMetric('LLM', stats.overall.by_category_llm[category])}  `
      + `n=${count}`,
    )
  }
}

export const printComparison = (comparison: BenchmarkComparisonSummary): void => {
  log.message([
    'delta overall  ',
    formatMetric('F1', comparison.overall.score_delta),
    '  ',
    formatMetric('NemoriF1', comparison.overall.nemori_f1_delta),
    '  ',
    formatMetric('LLM', comparison.overall.llm_judge_delta),
  ].join(''))

  for (const category of CATEGORIES) {
    const metric = comparison.by_category[category]
    log.message([
      '  c',
      String(category),
      ' ',
      CATEGORY_NAMES[category].padEnd(12),
      '  ',
      formatMetric('F1', metric.score_delta),
      '  ',
      formatMetric('NemoriF1', metric.nemori_f1_delta),
      '  ',
      formatMetric('LLM', metric.llm_judge_delta),
    ].join(''))
  }
}
