import type { QACategory } from './types'

import { env } from 'node:process'

import { generateText } from '@xsai/generate-text'

const SYSTEM_PROMPT = [
  'You answer questions by reading retrieved conversation memories and extracting the most accurate supported answer.',
].join('\n')

const buildPrompt = (context: string, question: string, category: QACategory): string => {
  const contextSection = context.length > 0
    ? `Conversation memories:\n${context}\n\n`
    : ''

  if (category === 5) {
    return `${contextSection}Answer the question using only the retrieved memories above.
- If the topic does not appear anywhere in those memories, reply exactly: "No information available"
- Keep the answer under 5-6 words

Question: ${question}
Short answer:`
  }

  return `${contextSection}# Context
The memories come from a conversation between two speakers.
Some of them include timestamps that may matter for answering the question.

# Instructions
1. Read all retrieved memories from both speakers carefully.
2. Pay close attention to timestamps when the answer depends on time.
3. If the question asks about a specific event or fact, look for direct support in the memories.
4. If the memories conflict, prefer the one with the more recent timestamp.
5. When a memory uses a relative time phrase such as "last year" or "two months ago", resolve it against that memory's timestamp.
   Example: if a memory dated 4 May 2022 says "went to India last year," then the trip happened in 2021.
6. Convert relative time references into a specific date, month, or year in the final answer. Do not answer with the relative phrase itself.
7. Base the answer only on the memory content from both speakers. If a name appears inside a memory, do not assume that person is the speaker who created it.
8. Keep the final answer under 6-7 words.

# Approach
1. Identify the memories that are relevant to the question.
2. Examine their timestamps and content carefully.
3. Look for explicit mentions of dates, times, locations, or events that answer the question.
4. If a calculation is required, work it out before answering.
5. Write a precise, concise answer supported only by the memories.
6. Check that the answer directly addresses the question.
7. Make sure the final answer is specific and avoids vague time references.

Question: ${question}
Short answer:`
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
