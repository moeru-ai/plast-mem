import type { LongMemEvalSample } from './types'

import { env } from 'node:process'

import { generateText } from '@xsai/generate-text'

export interface GenerateAnswerOptions {
  context: string
  model?: string
  question: string
  questionDate?: string
}

const getRequiredEnv = (key: 'OPENAI_API_KEY' | 'OPENAI_BASE_URL' | 'OPENAI_CHAT_MODEL'): string => {
  const value = env[key]
  if (value == null || value.length === 0) {
    throw new Error(`Missing required environment variable: ${key}`)
  }
  return value
}

const buildAnswerPrompt = ({
  context,
  question,
  questionDate,
}: GenerateAnswerOptions): string => [
  'You are answering a LongMemEval question using retrieved memory snippets.',
  'Use only the provided context.',
  'If the context is insufficient, answer "Insufficient information."',
  'Prefer the most recent fact when the context shows updates or contradictions.',
  'Answer briefly and directly. Do not explain your reasoning.',
  '',
  questionDate == null || questionDate.length === 0 ? '' : `Question date: ${questionDate}`,
  `Question: ${question}`,
  '',
  'Retrieved context:',
  context.length === 0 ? '(empty)' : context,
  '',
  'Answer:',
].filter(line => line.length > 0).join('\n')

export const generateAnswer = async ({
  context,
  model,
  question,
  questionDate,
}: GenerateAnswerOptions): Promise<string> => {
  const apiKey = getRequiredEnv('OPENAI_API_KEY')
  const baseURL = getRequiredEnv('OPENAI_BASE_URL')
  const resolvedModel = model ?? getRequiredEnv('OPENAI_CHAT_MODEL')

  const { text } = await generateText({
    apiKey,
    baseURL,
    maxTokens: 128,
    messages: [{
      content: buildAnswerPrompt({
        context,
        question,
        questionDate,
      }),
      role: 'user',
    }],
    model: resolvedModel,
    temperature: 0,
  })

  return text?.trim() ?? ''
}

export const generateSampleAnswer = async (
  sample: LongMemEvalSample,
  context: string,
  model?: string,
): Promise<string> =>
  generateAnswer({
    context,
    model,
    question: sample.improved_question ?? sample.question,
    questionDate: sample.question_date,
  })
