import type { LongMemEvalSample } from './types'

import { env } from 'node:process'

import { generateText } from '@xsai/generate-text'

export interface JudgeAnswerOptions {
  model?: string
  prediction: string
  sample: LongMemEvalSample
  seed?: number
}

export interface JudgeAnswerResult {
  score: 0 | 1
  verdict: string
}

const getRequiredEnv = (key: 'OPENAI_API_KEY' | 'OPENAI_BASE_URL' | 'OPENAI_CHAT_MODEL'): string => {
  const value = env[key]
  if (value == null || value.length === 0) {
    throw new Error(`Missing required environment variable: ${key}`)
  }
  return value
}

const buildDefaultPrompt = (
  sample: LongMemEvalSample,
  prediction: string,
): string => [
  'You are grading a LongMemEval answer.',
  'Return CORRECT if the predicted answer is semantically consistent with the gold answer.',
  'Return WRONG if the prediction contradicts the gold answer, misses required information, or invents unsupported detail.',
  'Accept equivalent wording and valid paraphrases.',
  'If the gold answer implies insufficient evidence, only accept answers that also clearly abstain.',
  'Respond with exactly one word: CORRECT or WRONG.',
  '',
  `Question type: ${sample.question_type}`,
  `Question date: ${sample.question_date}`,
  `Question: ${sample.question}`,
  `Gold answer: ${String(sample.answer)}`,
  `Predicted answer: ${prediction}`,
].join('\n')

const buildTemporalPrompt = (sample: LongMemEvalSample, prediction: string): string => [
  'You are grading a LongMemEval temporal reasoning answer.',
  'Return CORRECT if the predicted answer contains the correct time, date, duration, or chronology.',
  'Accept equivalent wording, intermediate reasoning that leads to the correct answer, and off-by-one answers for counts of days/weeks/months when the intent is clearly correct.',
  'Return WRONG if the answer picks the wrong event, wrong direction in time, or lacks the key temporal fact.',
  'Respond with exactly one word: CORRECT or WRONG.',
  '',
  `Question date: ${sample.question_date}`,
  `Question: ${sample.question}`,
  `Gold answer: ${String(sample.answer)}`,
  `Predicted answer: ${prediction}`,
].join('\n')

const buildKnowledgeUpdatePrompt = (sample: LongMemEvalSample, prediction: string): string => [
  'You are grading a LongMemEval knowledge-update answer.',
  'Return CORRECT if the prediction includes the latest correct state, even if it also mentions older superseded information.',
  'Return WRONG if it only gives outdated information, misses the updated state, or invents unsupported changes.',
  'Respond with exactly one word: CORRECT or WRONG.',
  '',
  `Question date: ${sample.question_date}`,
  `Question: ${sample.question}`,
  `Gold answer: ${String(sample.answer)}`,
  `Predicted answer: ${prediction}`,
].join('\n')

const buildPreferencePrompt = (sample: LongMemEvalSample, prediction: string): string => [
  'You are grading a LongMemEval preference answer.',
  'Return CORRECT if the response correctly recalls and uses the user\'s personal preferences or profile information.',
  'The prediction does not need to match the gold answer word-for-word or mention every preference.',
  'Return WRONG if it recalls the wrong preference, misses the key personal information, or invents unsupported details.',
  'Respond with exactly one word: CORRECT or WRONG.',
  '',
  `Question date: ${sample.question_date}`,
  `Question: ${sample.question}`,
  `Gold answer or rubric: ${String(sample.answer)}`,
  `Predicted answer: ${prediction}`,
].join('\n')

const buildJudgePrompt = (sample: LongMemEvalSample, prediction: string): string => {
  if (sample.question_type === 'temporal-reasoning')
    return buildTemporalPrompt(sample, prediction)

  if (sample.question_type === 'knowledge-update')
    return buildKnowledgeUpdatePrompt(sample, prediction)

  if (sample.question_type === 'single-session-preference')
    return buildPreferencePrompt(sample, prediction)

  return buildDefaultPrompt(sample, prediction)
}

export const judgeAnswer = async ({
  model,
  prediction,
  sample,
  seed,
}: JudgeAnswerOptions): Promise<JudgeAnswerResult> => {
  const apiKey = getRequiredEnv('OPENAI_API_KEY')
  const baseURL = getRequiredEnv('OPENAI_BASE_URL')
  const resolvedModel = model ?? getRequiredEnv('OPENAI_CHAT_MODEL')

  const { text } = await generateText({
    apiKey,
    baseURL,
    maxTokens: 8,
    messages: [{
      content: buildJudgePrompt(sample, prediction),
      role: 'user',
    }],
    model: resolvedModel,
    seed,
    temperature: 0,
  })

  const verdict = text?.trim().toUpperCase() ?? ''

  return {
    score: verdict.startsWith('CORRECT') ? 1 : 0,
    verdict,
  }
}
