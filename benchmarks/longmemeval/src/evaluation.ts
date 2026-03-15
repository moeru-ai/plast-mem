import type { LongMemEvalSample } from './types'

import { env } from 'node:process'

import { generateText } from '@xsai/generate-text'

export interface JudgeAnswerOptions {
  model?: string
  prediction: string
  sample: LongMemEvalSample
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

const buildJudgePrompt = (
  sample: LongMemEvalSample,
  prediction: string,
): string => [
  'You are grading a LongMemEval answer.',
  'Return CORRECT if the predicted answer is semantically consistent with the gold answer.',
  'Return WRONG if the prediction contradicts the gold answer, misses the key fact, or invents unsupported detail.',
  'Accept equivalent wording and valid paraphrases.',
  'For questions involving updates or current state, prefer the latest correct state.',
  'If the gold answer implies insufficient evidence, only accept answers that also clearly abstain.',
  'Respond with exactly one word: CORRECT or WRONG.',
  '',
  `Question type: ${sample.question_type}`,
  `Question date: ${sample.question_date}`,
  `Question: ${sample.improved_question ?? sample.question}`,
  `Gold answer: ${String(sample.improved_answer ?? sample.answer)}`,
  `Predicted answer: ${prediction}`,
].join('\n')

export const judgeAnswer = async ({
  model,
  prediction,
  sample,
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
    temperature: 0,
  })

  const verdict = text?.trim().toUpperCase() ?? ''

  return {
    score: verdict.startsWith('CORRECT') ? 1 : 0,
    verdict,
  }
}
