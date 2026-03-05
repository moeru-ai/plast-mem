import type { QACategory } from './types'

import process from 'node:process'

import { generateText } from '@xsai/generate-text'

// ──────────────────────────────────────────────────
// Text normalization (mirrors LobeHub evaluation.py)
// ──────────────────────────────────────────────────

const ARTICLES = new Set(['a', 'an', 'and', 'the'])

const normalizeAnswer = (s: number | string): string =>
  String(s)
    .toLowerCase()
    .replace(/[^a-z0-9\s]/g, ' ')
    .split(/\s+/)
    .filter(w => w.length > 0 && !ARTICLES.has(w))
    .join(' ')

// ──────────────────────────────────────────────────
// Token-level F1
// ──────────────────────────────────────────────────

const tokenF1 = (prediction: string, groundTruth: string): number => {
  const predTokens = normalizeAnswer(prediction).split(' ').filter(token => token.length > 0)
  const goldTokens = normalizeAnswer(groundTruth).split(' ').filter(token => token.length > 0)

  if (predTokens.length === 0 && goldTokens.length === 0)
    return 1.0
  if (predTokens.length === 0 || goldTokens.length === 0)
    return 0.0

  const goldCount = new Map<string, number>()
  for (const t of goldTokens) goldCount.set(t, (goldCount.get(t) ?? 0) + 1)

  let numSame = 0
  for (const t of predTokens) {
    const cnt = goldCount.get(t) ?? 0
    if (cnt > 0) {
      numSame++
      goldCount.set(t, cnt - 1)
    }
  }

  if (numSame === 0)
    return 0.0

  const precision = numSame / predTokens.length
  const recall = numSame / goldTokens.length
  return (2 * precision * recall) / (precision + recall)
}

// ──────────────────────────────────────────────────
// Per-category scoring (mirrors LobeHub evaluation.py)
// ──────────────────────────────────────────────────

/**
 * Category 1 – multi-hop:
 * Gold answer may be comma-separated sub-answers.
 * Score = mean over sub-answers of max F1 against each prediction token.
 */
const scoreCategory1 = (prediction: string, goldAnswer: string): number => {
  const subAnswers = goldAnswer.split(',').map(s => s.trim()).filter(sub => sub.length > 0)
  if (subAnswers.length === 0)
    return 0.0
  const scores = subAnswers.map(sub => tokenF1(prediction, sub))
  return scores.reduce((a, b) => a + b, 0) / scores.length
}

/**
 * Category 3 – temporal:
 * Only the first semicolon-delimited part of the gold answer is used.
 */
const scoreCategory3 = (prediction: string, goldAnswer: string): number => {
  const gold = goldAnswer.split(';')[0]?.trim() ?? goldAnswer
  return tokenF1(prediction, gold)
}

/**
 * Category 5 – adversarial:
 * Binary score: 1 if prediction signals absence of information, 0 otherwise.
 */
const scoreCategory5 = (prediction: string): number => {
  const lower = prediction.toLowerCase()
  return lower.includes('no information') || lower.includes('not mentioned') ? 1.0 : 0.0
}

/**
 * Score a single prediction against the gold answer for a given category.
 */
export const scoreAnswer = (
  prediction: string,
  goldAnswer: number | string,
  category: QACategory,
): number => {
  const gold = String(goldAnswer)
  switch (category) {
    case 1:
      return scoreCategory1(prediction, gold)
    case 2:
    case 4:
      return tokenF1(prediction, gold)
    case 3:
      return scoreCategory3(prediction, gold)
    case 5:
      return scoreCategory5(prediction)
  }
}

// ──────────────────────────────────────────────────
// LLM Judge (lenient semantic evaluation)
// ──────────────────────────────────────────────────

export const llmJudge = async (
  prediction: string,
  goldAnswer: number | string,
  question: string,
  model: string,
): Promise<number> => {
  const gold = String(goldAnswer)
  const prompt = `Question: ${question}
Gold answer: ${gold}
Predicted answer: ${prediction}

Is the predicted answer correct? Guidelines:
- Accept semantically equivalent answers (e.g., "adoption agency" ≈ "adoption agencies")
- Accept if a relative time expression in the prediction matches the specific date in the gold
- Accept if the prediction captures the key fact even if phrased differently
- For adversarial questions (gold = "No information available"): only accept if prediction also signals no information

Respond with exactly one word: CORRECT or WRONG`

  const { text } = await generateText({
    apiKey: process.env.OPENAI_API_KEY ?? '',
    baseURL: process.env.OPENAI_BASE_URL ?? 'https://api.openai.com/v1',
    maxTokens: 10,
    messages: [{ content: prompt, role: 'user' }],
    model,
    temperature: 0,
  })

  return (text ?? '').trim().toUpperCase().startsWith('CORRECT') ? 1.0 : 0.0
}
