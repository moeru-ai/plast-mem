import type { QACategory } from './types'

import { env } from 'node:process'

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
// Nemori-style F1
// ──────────────────────────────────────────────────

const simpleTokenizeNemori = (text: string): string[] =>
  text
    .toLowerCase()
    .replace(/[.,!?]/g, ' ')
    .split(/\s+/)
    .filter(token => token.length > 0)

export const scoreAnswerNemoriF1 = (
  prediction: string,
  goldAnswer: number | string,
): number => {
  const normalizedPrediction = String(prediction).trim()
  const normalizedGold = String(goldAnswer).trim()

  if (normalizedPrediction.length === 0 || normalizedGold.length === 0)
    return 0.0

  const predTokens = new Set(simpleTokenizeNemori(normalizedPrediction))
  const goldTokens = new Set(simpleTokenizeNemori(normalizedGold))

  if (predTokens.size === 0 || goldTokens.size === 0)
    return 0.0

  let commonCount = 0
  for (const token of predTokens) {
    if (goldTokens.has(token))
      commonCount += 1
  }

  if (commonCount === 0)
    return 0.0

  const precision = commonCount / predTokens.size
  const recall = commonCount / goldTokens.size

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

const CATEGORY_NAMES: Record<QACategory, string> = {
  1: 'multi-hop',
  2: 'temporal',
  3: 'open-domain',
  4: 'single-hop',
  5: 'adversarial',
}

const buildJudgeGuidance = (category: QACategory): string => {
  switch (category) {
    case 1:
      return [
        'This is a multi-hop fact question.',
        'Use three labels: CORRECT, PARTIAL, WRONG.',
        'Label CORRECT only if the prediction covers all essential pieces required by the question.',
        'Label PARTIAL if the prediction captures a substantial and clearly relevant part of the answer but misses one or more required pieces.',
        'Label WRONG if it misses the main answer, gives the wrong facts, or mixes in mostly unrelated items.',
        'Accept concise phrasing and semantically equivalent wording, but do not accept incomplete coverage.',
        'Do not label CORRECT if a two-part or multi-part question is answered only partially.',
      ].join('\n')
    case 2:
      return [
        'This is a temporal question.',
        'Use only CORRECT or WRONG.',
        'Accept semantically equivalent time expressions, including absolute vs relative phrasing, if they refer to the same date, time period, duration, or ordering.',
        'Do not require the exact wording of the gold answer.',
        'Label the answer WRONG if it refers to the wrong date, wrong time period, wrong duration, or wrong sequence of events.',
      ].join('\n')
    case 3:
      return [
        'This is an open-domain question.',
        'Use three labels: CORRECT, PARTIAL, WRONG.',
        'Judge whether the prediction captures the same core idea as the gold answer.',
        'Label CORRECT if the main idea matches well.',
        'Label PARTIAL if the prediction is broadly on the right topic but only captures part of the intended meaning.',
        'Label WRONG if it misses the main point, introduces a different conclusion, or relies on unsupported claims.',
      ].join('\n')
    case 4:
      return [
        'This is a single-hop factual question.',
        'Use three labels: CORRECT, PARTIAL, WRONG.',
        'Accept answers that identify the same entity, object, title, place, or fact as the gold answer, even if phrased differently or embedded in a longer sentence.',
        'Label CORRECT if the prediction identifies the right specific fact or entity.',
        'Label PARTIAL if the prediction is close but too broad, incomplete, or only partially specific.',
        'Label WRONG if it names the wrong entity or replaces a specific answer with a materially different broader concept.',
      ].join('\n')
    case 5:
      return [
        'This is an adversarial question.',
        'Use only CORRECT or WRONG.',
        'Label the answer CORRECT only if it clearly conveys that the information is not mentioned or cannot be answered from the conversation.',
        'Any concrete factual answer should be labeled WRONG.',
      ].join('\n')
  }
}

const buildJudgePrompt = (
  question: string,
  goldAnswer: number | string,
  prediction: string,
  category: QACategory,
): string => `You are an expert evaluator for long-conversation question answering.

Task category: ${CATEGORY_NAMES[category]}

${buildJudgeGuidance(category)}

Question: ${question}
Gold answer: ${String(goldAnswer)}
Predicted answer: ${prediction}

Return exactly one word:
CORRECT
or
PARTIAL
or
WRONG`

const parseJudgeLabel = (text: null | string | undefined): number => {
  const normalized = (text ?? '').trim().toUpperCase()
  if (normalized === 'CORRECT')
    return 1.0
  if (normalized === 'PARTIAL')
    return 0.5
  if (normalized === 'WRONG')
    return 0.0

  const firstWord = normalized.split(/\s+/)[0] ?? ''
  if (firstWord === 'CORRECT')
    return 1.0
  if (firstWord === 'PARTIAL')
    return 0.5
  if (firstWord === 'WRONG')
    return 0.0

  return 0.0
}

export const llmJudge = async (
  prediction: string,
  goldAnswer: number | string,
  question: string,
  category: QACategory,
  model: string,
): Promise<number> => {
  const prompt = buildJudgePrompt(question, goldAnswer, prediction, category)

  const { text } = await generateText({
    apiKey: env.OPENAI_API_KEY ?? '',
    baseURL: env.OPENAI_BASE_URL ?? 'https://api.openai.com/v1',
    maxTokens: 16,
    messages: [{ content: prompt, role: 'user' }],
    model,
    reasoningEffort: 'none',
    temperature: 0,
  })

  return parseJudgeLabel(text)
}
