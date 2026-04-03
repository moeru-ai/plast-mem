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

const formatPercent = (value: number): string => `${(value * 100).toFixed(2)}%`

const formatDeltaPercent = (value: number): string => `${value >= 0 ? '+' : ''}${(value * 100).toFixed(2)}%`

const formatInlineSummary = (summary: BenchmarkScoreSummary): string =>
  `F1 ${formatPercent(summary.overall)}  `
  + `NemoriF1 ${formatPercent(summary.overall_nemori_f1)}  `
  + `LLM ${formatPercent(summary.overall_llm)}`

const formatInlineDelta = (metric: BenchmarkComparisonMetric): string =>
  `F1 ${formatDeltaPercent(metric.score_delta)}  `
  + `NemoriF1 ${formatDeltaPercent(metric.nemori_f1_delta)}  `
  + `LLM ${formatDeltaPercent(metric.llm_judge_delta)}`

const padCell = (value: string, width: number, align: 'left' | 'right' = 'left'): string =>
  align === 'right' ? value.padStart(width) : value.padEnd(width)

const buildTable = (
  headers: string[],
  rows: string[][],
  alignments: Array<'left' | 'right'>,
): string => {
  const widths = headers.map((header, columnIndex) =>
    Math.max(
      header.length,
      ...rows.map(row => (row[columnIndex] ?? '').length),
    ),
  )

  const headerLine = headers
    .map((header, columnIndex) => padCell(header, widths[columnIndex], alignments[columnIndex]))
    .join('  ')
  const separatorLine = widths.map(width => '-'.repeat(width)).join('  ')
  const bodyLines = rows.map(row =>
    row.map((cell, columnIndex) => padCell(cell, widths[columnIndex], alignments[columnIndex])).join('  '))

  return [headerLine, separatorLine, ...bodyLines].join('\n')
}

const buildMarkdownTable = (
  headers: string[],
  rows: string[][],
  alignments: Array<'left' | 'right'>,
): string => {
  const alignmentRow = alignments.map(alignment => alignment === 'right' ? '---:' : ':---')
  const lines = [
    `| ${headers.join(' | ')} |`,
    `| ${alignmentRow.join(' | ')} |`,
    ...rows.map(row => `| ${row.join(' | ')} |`),
  ]
  return lines.join('\n')
}

export const renderStats = (stats: BenchmarkStats): string => {
  const sections: string[] = []

  sections.push('OVERALL')
  sections.push(buildTable(
    ['split', 'F1', 'NemoriF1', 'LLM', 'n'],
    [[
      'overall',
      formatPercent(stats.overall.overall),
      formatPercent(stats.overall.overall_nemori_f1),
      formatPercent(stats.overall.overall_llm),
      String(stats.overall.total),
    ]],
    ['left', 'right', 'right', 'right', 'right'],
  ))

  const sampleRows = Object.entries(stats.by_sample)
    .map(([sampleId, summary]) => [
      sampleId,
      formatPercent(summary.overall),
      formatPercent(summary.overall_nemori_f1),
      formatPercent(summary.overall_llm),
      String(summary.total),
    ])

  if (sampleRows.length > 0) {
    sections.push('SAMPLES')
    sections.push(buildTable(
      ['sample', 'F1', 'NemoriF1', 'LLM', 'n'],
      sampleRows,
      ['left', 'right', 'right', 'right', 'right'],
    ))
  }

  const categoryRows = CATEGORIES
    .filter(category => stats.overall.by_category_count[category] > 0)
    .map(category => [
      `c${category}`,
      CATEGORY_NAMES[category],
      formatPercent(stats.overall.by_category[category]),
      formatPercent(stats.overall.by_category_nemori_f1[category]),
      formatPercent(stats.overall.by_category_llm[category]),
      String(stats.overall.by_category_count[category]),
    ])

  sections.push('CATEGORIES')
  sections.push(buildTable(
    ['id', 'category', 'F1', 'NemoriF1', 'LLM', 'n'],
    categoryRows,
    ['left', 'left', 'right', 'right', 'right', 'right'],
  ))

  return sections.join('\n\n')
}

export const renderStatsMarkdown = (stats: BenchmarkStats): string => {
  const sections: string[] = []

  sections.push('### Overall')
  sections.push(buildMarkdownTable(
    ['split', 'F1', 'NemoriF1', 'LLM', 'n'],
    [[
      'overall',
      formatPercent(stats.overall.overall),
      formatPercent(stats.overall.overall_nemori_f1),
      formatPercent(stats.overall.overall_llm),
      String(stats.overall.total),
    ]],
    ['left', 'right', 'right', 'right', 'right'],
  ))

  const sampleRows = Object.entries(stats.by_sample)
    .map(([sampleId, summary]) => [
      sampleId,
      formatPercent(summary.overall),
      formatPercent(summary.overall_nemori_f1),
      formatPercent(summary.overall_llm),
      String(summary.total),
    ])

  if (sampleRows.length > 0) {
    sections.push('### Samples')
    sections.push(buildMarkdownTable(
      ['sample', 'F1', 'NemoriF1', 'LLM', 'n'],
      sampleRows,
      ['left', 'right', 'right', 'right', 'right'],
    ))
  }

  const categoryRows = CATEGORIES
    .filter(category => stats.overall.by_category_count[category] > 0)
    .map(category => [
      `c${category}`,
      CATEGORY_NAMES[category],
      formatPercent(stats.overall.by_category[category]),
      formatPercent(stats.overall.by_category_nemori_f1[category]),
      formatPercent(stats.overall.by_category_llm[category]),
      String(stats.overall.by_category_count[category]),
    ])

  sections.push('### Categories')
  sections.push(buildMarkdownTable(
    ['id', 'category', 'F1', 'NemoriF1', 'LLM', 'n'],
    categoryRows,
    ['left', 'left', 'right', 'right', 'right', 'right'],
  ))

  return sections.join('\n\n')
}

export const renderComparison = (comparison: BenchmarkComparisonSummary): string => {
  const sections: string[] = []

  sections.push('OVERALL DELTA')
  sections.push(buildTable(
    ['metric', 'delta'],
    [
      ['F1', formatDeltaPercent(comparison.overall.score_delta)],
      ['NemoriF1', formatDeltaPercent(comparison.overall.nemori_f1_delta)],
      ['LLM', formatDeltaPercent(comparison.overall.llm_judge_delta)],
    ],
    ['left', 'right'],
  ))

  sections.push('CATEGORY DELTA')
  sections.push(buildTable(
    ['id', 'category', 'F1', 'NemoriF1', 'LLM'],
    CATEGORIES.filter(category => category !== 5).map(category => [
      `c${category}`,
      CATEGORY_NAMES[category],
      formatDeltaPercent(comparison.by_category[category].score_delta),
      formatDeltaPercent(comparison.by_category[category].nemori_f1_delta),
      formatDeltaPercent(comparison.by_category[category].llm_judge_delta),
    ]),
    ['left', 'left', 'right', 'right', 'right'],
  ))

  return sections.join('\n\n')
}

export const renderComparisonMarkdown = (comparison: BenchmarkComparisonSummary): string => {
  const sections: string[] = []

  sections.push('### Overall Delta')
  sections.push(buildMarkdownTable(
    ['metric', 'delta'],
    [
      ['F1', formatDeltaPercent(comparison.overall.score_delta)],
      ['NemoriF1', formatDeltaPercent(comparison.overall.nemori_f1_delta)],
      ['LLM', formatDeltaPercent(comparison.overall.llm_judge_delta)],
    ],
    ['left', 'right'],
  ))

  sections.push('### Category Delta')
  sections.push(buildMarkdownTable(
    ['id', 'category', 'F1', 'NemoriF1', 'LLM'],
    CATEGORIES.filter(category => category !== 5).map(category => [
      `c${category}`,
      CATEGORY_NAMES[category],
      formatDeltaPercent(comparison.by_category[category].score_delta),
      formatDeltaPercent(comparison.by_category[category].nemori_f1_delta),
      formatDeltaPercent(comparison.by_category[category].llm_judge_delta),
    ]),
    ['left', 'left', 'right', 'right', 'right'],
  ))

  return sections.join('\n\n')
}

export const renderSampleSummaryBlock = (
  plastmem: BenchmarkScoreSummary,
  fullContext: BenchmarkScoreSummary,
  delta?: BenchmarkComparisonMetric,
): string => {
  const lines = [
    `plast-mem     ${formatInlineSummary(plastmem)}`,
    `full-context  ${formatInlineSummary(fullContext)}`,
  ]

  if (delta != null)
    lines.push(`delta         ${formatInlineDelta(delta)}`)

  return lines.join('\n')
}

export const printStats = (stats: BenchmarkStats): void => {
  log.message(renderStats(stats))
}

export const printComparison = (comparison: BenchmarkComparisonSummary): void => {
  log.message(renderComparison(comparison))
}
