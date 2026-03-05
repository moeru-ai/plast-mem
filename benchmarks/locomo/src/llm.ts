import type { QACategory } from './types'

import { env } from 'node:process'

import { generateText } from '@xsai/generate-text'

const SYSTEM_PROMPT
  = 'You are a helpful assistant answering questions about a person based on their conversation history stored in memory.'

const buildPrompt = (context: string, question: string, category: QACategory): string => {
  const contextSection = context.length > 0
    ? `Conversation memories:\n${context}\n\n`
    : ''

  if (category === 5) {
    return `${contextSection}Answer the following question using only the memories above. If this topic is not mentioned anywhere in the memories, respond with exactly: "No information available"\n\nQuestion: ${question}\nShort answer:`
  }

  return `${contextSection}Answer the following question based on the memories above.\n- Answer in a short phrase (under 10 words)\n- Use exact words from the memories when possible\n- Memories include timestamps; use them to resolve relative time expressions (e.g., if a memory says "yesterday" in a session dated "8 May 2023", the answer is "7 May 2023")\n\nQuestion: ${question}\nShort answer:`
}

/**
 * Generate an answer for a single QA pair.
 */
export const generateAnswer = async (
  context: string,
  question: string,
  category: QACategory,
  model = 'gpt-4o-mini',
): Promise<string> => {
  const prompt = buildPrompt(context, question, category)

  const { text } = await generateText({
    apiKey: env.OPENAI_API_KEY ?? '',
    baseURL: env.OPENAI_BASE_URL ?? 'https://api.openai.com/v1',
    maxTokens: 200,
    messages: [
      { content: SYSTEM_PROMPT, role: 'system' },
      { content: prompt, role: 'user' },
    ],
    model,
    temperature: 0,
  })

  return text ?? ''
}
