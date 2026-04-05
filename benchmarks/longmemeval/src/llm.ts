import type { LongMemEvalSample } from './types'

import { env } from 'node:process'

import { generateText } from '@xsai/generate-text'

export interface GenerateAnswerOptions {
  context: string
  model?: string
  question: string
  questionDate?: string
  questionType?: LongMemEvalSample['question_type']
  seed?: number
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
  questionType,
}: GenerateAnswerOptions): string => [
  'You are a memory benchmark assistant answering a LongMemEval question from retrieved memories.',
  'Use only the retrieved context. Do not invent facts, dates, names, counts, or recommendations.',
  'If the retrieved context does not contain enough evidence, answer "Insufficient information."',
  'When memories conflict, prefer the most recent supported state.',
  'Convert relative time references to absolute dates or years before answering.',
  'For counting questions, scan all relevant memories, deduplicate repeated references, and answer with the final count only.',
  'For comparison questions, if one side is missing from the evidence, say that the information is insufficient.',
  'For recommendation or preference questions, describe the kind of thing the user would prefer based on memory; do not fabricate specific recommendations.',
  'Keep the final answer concise and direct. Do not explain your reasoning unless the question requires a computed value.',
  '',
  questionType == null ? '' : `Question type: ${questionType}`,
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
  questionType,
  seed,
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
        questionType,
      }),
      role: 'user',
    }],
    model: resolvedModel,
    seed,
    temperature: 0,
  })

  return text?.trim() ?? ''
}

export const generateSampleAnswer = async (
  sample: LongMemEvalSample,
  context: string,
  model?: string,
  seed?: number,
): Promise<string> =>
  generateAnswer({
    context,
    model,
    question: sample.question,
    questionDate: sample.question_date,
    questionType: sample.question_type,
    seed,
  })
