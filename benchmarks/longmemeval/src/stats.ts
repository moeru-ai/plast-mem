import type { LongMemEvalQuestionType, LongMemEvalResult, LongMemEvalStats } from './types'

const QUESTION_TYPES: LongMemEvalQuestionType[] = [
  'knowledge-update',
  'multi-session',
  'single-session-assistant',
  'single-session-preference',
  'single-session-user',
  'temporal-reasoning',
]

const avg = (scores: number[]): number =>
  scores.length > 0 ? scores.reduce((a, b) => a + b, 0) / scores.length : 0

export const computeStats = (results: LongMemEvalResult[]): LongMemEvalStats => {
  const byQuestionType = Object.fromEntries(
    QUESTION_TYPES.map(type => [type, [] as number[]]),
  ) as Record<LongMemEvalQuestionType, number[]>

  for (const result of results)
    byQuestionType[result.question_type].push(result.score)

  return {
    by_question_type: Object.fromEntries(
      QUESTION_TYPES.map(type => [type, avg(byQuestionType[type])]),
    ) as Record<LongMemEvalQuestionType, number>,
    by_question_type_count: Object.fromEntries(
      QUESTION_TYPES.map(type => [type, byQuestionType[type].length]),
    ) as Record<LongMemEvalQuestionType, number>,
    overall: avg(results.map(result => result.score)),
    total: results.length,
  }
}
