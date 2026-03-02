import type { BenchmarkStats, QACategory, QAResult } from './types'

import process from 'node:process'

const CATEGORIES: QACategory[] = [1, 2, 3, 4, 5]
const CATEGORY_NAMES: Record<QACategory, string> = {
  1: 'multi-hop',
  2: 'single-hop',
  3: 'temporal',
  4: 'open-domain',
  5: 'adversarial',
}

export const computeStats = (results: QAResult[]): BenchmarkStats => {
  const byCategory = Object.fromEntries(
    CATEGORIES.map(c => [c, [] as number[]]),
  ) as Record<QACategory, number[]>

  for (const r of results) {
    byCategory[r.category].push(r.score)
  }

  const categoryAvg = Object.fromEntries(
    CATEGORIES.map((c) => {
      const scores = byCategory[c]
      return [c, scores.length > 0 ? scores.reduce((a, b) => a + b, 0) / scores.length : 0]
    }),
  ) as Record<QACategory, number>

  const categoryCount = Object.fromEntries(
    CATEGORIES.map(c => [c, byCategory[c].length]),
  ) as Record<QACategory, number>

  const allScores = results.map(r => r.score)
  const overall = allScores.length > 0
    ? allScores.reduce((a, b) => a + b, 0) / allScores.length
    : 0

  return {
    by_category: categoryAvg,
    by_category_count: categoryCount,
    overall,
    total: results.length,
  }
}

export const printStats = (stats: BenchmarkStats): void => {
  process.stdout.write('\n── Results ──────────────────────────────────\n')
  process.stdout.write(`Overall F1:  ${(stats.overall * 100).toFixed(2)}%  (n=${stats.total})\n`)
  process.stdout.write('\n')
  for (const c of CATEGORIES) {
    const avg = stats.by_category[c]
    const count = stats.by_category_count[c]
    if (count > 0) {
      process.stdout.write(
        `  Cat ${c} (${CATEGORY_NAMES[c].padEnd(12)}):  ${(avg * 100).toFixed(2)}%  (n=${count})\n`,
      )
    }
  }
  process.stdout.write('──────────────────────────────────────────────\n\n')
}
