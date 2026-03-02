import type { QACategory } from './types'

import process from 'node:process'

import { generateText } from '@xsai/generate-text'

// Prompt templates matching LobeHub evaluate_qa.py / gpt_utils.py
const SYSTEM_PROMPT_DEFAULT
  = 'You are a helpful assistant that answers questions based on provided conversation context.'

const buildPrompt = (context: string, question: string, category: QACategory): string => {
  const contextSection = context
    ? `Context from memory:\n${context}\n\n`
    : 'No relevant context was found in memory.\n\n'

  if (category === 5) {
    return (
      `${contextSection
      }Based on the above context, answer the following question. `
      + 'If the information is not available in the context, respond with exactly: "No information available"\n\n'
      + `Question: ${question}\nShort answer:`
    )
  }

  return (
    `${contextSection
    }Based on the above context, write an answer in the form of a short phrase for the following question. `
    + 'Answer with exact words from the context whenever possible.\n\n'
    + `Question: ${question}\nShort answer:`
  )
}

/**
 * Generate an answer for a single QA pair using the retrieved context.
 */
export const generateAnswer = async (
  context: string,
  question: string,
  category: QACategory,
  model = 'gpt-4o-mini',
): Promise<string> => {
  const prompt = buildPrompt(context, question, category)

  const { text } = await generateText({
    apiKey: process.env.OPENAI_API_KEY ?? '',
    baseURL: process.env.OPENAI_BASE_URL ?? 'https://api.openai.com/v1',
    maxTokens: 100,
    messages: [
      { content: SYSTEM_PROMPT_DEFAULT, role: 'system' },
      { content: prompt, role: 'user' },
    ],
    model,
    temperature: 0,
  })

  return text ?? ''
}
