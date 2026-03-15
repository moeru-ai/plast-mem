/* eslint-disable no-console */
import type { BenchmarkScoreSummary, BenchmarkStats, QACategory, QAResult } from './types'

const CATEGORIES: QACategory[] = [1, 2, 3, 4, 5]
const CATEGORY_NAMES: Record<QACategory, string> = {
  1: 'multi-hop',
  2: 'single-hop',
  3: 'temporal',
  4: 'open-domain',
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
    if (sampleResults == null) {
      resultsBySample.set(result.sample_id, [result])
    } else {
      sampleResults.push(result)
    }
  }

  const bySample = Object.fromEntries(
    [...resultsBySample.entries()]
      .sort(([sampleA], [sampleB]) => sampleA.localeCompare(sampleB))
      .map(([sampleId, sampleResults]) => [sampleId, computeScoreSummary(sampleResults)]),
  ) as Record<string, BenchmarkScoreSummary>

  return {
    by_sample: bySample,
    overall: computeScoreSummary(results),
  }
}

const printScoreSummary = (label: string, summary: BenchmarkScoreSummary): void => {
  console.log(`${label} F1:   ${(summary.overall * 100).toFixed(2)}%  (n=${summary.total})`)
  console.log(`${label} Nemori F1: ${(summary.overall_nemori_f1 * 100).toFixed(2)}%`)
  console.log(`${label} LLM:  ${(summary.overall_llm * 100).toFixed(2)}%`)
  console.log()

  for (const c of CATEGORIES) {
    const f1 = summary.by_category[c]
    const llm = summary.by_category_llm[c]
    const nemoriF1 = summary.by_category_nemori_f1[c]
    const count = summary.by_category_count[c]
    if (count > 0) {
      console.log(
        `  Cat ${c} (${CATEGORY_NAMES[c].padEnd(12)}):  F1=${(f1 * 100).toFixed(2)}%  NemoriF1=${(nemoriF1 * 100).toFixed(2)}%  LLM=${(llm * 100).toFixed(2)}%  (n=${count})`,
      )
    }
  }
}

export const printStats = (stats: BenchmarkStats): void => {
  console.log('\n── Results ──────────────────────────────────')
  const sampleIds = Object.keys(stats.by_sample)

  if (sampleIds.length > 0) {
    console.log('By sample:')
    for (const sampleId of sampleIds) {
      console.log()
      console.log(`Sample ${sampleId}`)
      printScoreSummary('  Overall', stats.by_sample[sampleId])
    }
  }

  console.log()
  console.log('Overall')
  printScoreSummary('  Overall', stats.overall)
  console.log('──────────────────────────────────────────────\n')
}
