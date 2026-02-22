import type { AssistantMessage, Message, UserMessage } from '@xsai/shared-chat'
import type { AddMessageMessage } from 'plastmem'

import { env } from 'node:process'

import useSWR from 'swr'
import useSWRMutation from 'swr/mutation'
import z from 'zod'

import { generateText } from '@xsai/generate-text'
import { tool } from '@xsai/tool'
import { addMessage, recentMemoryRaw, retrieveMemory } from 'plastmem'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Temporal } from 'temporal-polyfill'

import { prompt } from '../core/prompt'

const durationFormat = new Intl.DurationFormat('en', { style: 'narrow' })

const DEFAULT_TOKEN_BUDGET = 8192
const CONTEXT_WINDOW_RATIO = 0.2

/** Approximate token count: 4 chars ≈ 1 token */
function approxTokens(text: string): number {
  return Math.ceil(text.length / 4)
}

/** Truncate messages from the front to fit within token budget */
function truncateMessages(messages: Message[], budget: number): Message[] {
  let total = 0
  const result: Message[] = []
  for (let i = messages.length - 1; i >= 0; i--) {
    const tokens = approxTokens((messages[i] as { content: string }).content)
    if (total + tokens > budget)
      break
    result.unshift(messages[i])
    total += tokens
  }
  return result
}

export const useHaru = (conversation_id: string) => {
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const initialAt = useMemo(() => Temporal.Now.instant(), [conversation_id])

  const [messages, setMessages] = useState<Message[]>([])
  const messagesRef = useRef<Message[]>([])

  const clear = useCallback(() => {
    messagesRef.current = []
    // eslint-disable-next-line react-hooks-extra/no-direct-set-state-in-use-effect
    setMessages([])
  }, [])

  useEffect(() => clear(), [clear, conversation_id])

  const { data: tokenBudget } = useSWR('haru/tokenBudget', async () => {
    try {
      const res = await fetch(`${env.OPENAI_BASE_URL}/models`, {
        headers: { Authorization: `Bearer ${env.OPENAI_API_KEY}` },
      })
      const json = await res.json() as { data: { id: string, context_length?: number }[] }
      const model = json.data.find(m => m.id === env.OPENAI_CHAT_MODEL)
      if (model?.context_length)
        return Math.floor(model.context_length * CONTEXT_WINDOW_RATIO)
    }
    catch {}
    return DEFAULT_TOKEN_BUDGET
  }, { revalidateOnFocus: false })

  const { data: tools, isLoading: isToolsLoading } = useSWR(['haru/tools', conversation_id], async () => {
    const retrieveMemoryTool = await tool({
      description: 'Search long-term memory for relevant facts and past episodes',
      execute: async ({ query }) => retrieveMemory({ body: { conversation_id, query } }).then(res => res.data ?? String(res.error)),
      name: 'retrieve_memory',
      parameters: z.object({
        query: z.string().describe('Search query'),
      }),
    })
    return [retrieveMemoryTool]
  }, { revalidateOnFocus: false })

  const { data: episodicMemory, isLoading: isMemoryLoading } = useSWR(
    ['haru/recentMemory', conversation_id],
    async () => recentMemoryRaw({ body: { conversation_id } }).then(res => res.data),
  )

  const pushMessage = useCallback(async (message: AssistantMessage | UserMessage) => {
    messagesRef.current = [...messagesRef.current, message]
    setMessages(prev => [...prev, message])
    await addMessage({ body: { conversation_id, message: message as AddMessageMessage } })
  }, [conversation_id])

  const { error, isMutating, trigger: send } = useSWRMutation<void, Error, [string, string], string>(
    ['haru/send', conversation_id],
    async (_, { arg: input }) => {
      if (input.trim().length === 0)
        return

      await pushMessage({ content: input, role: 'user' })

      const now = Temporal.Now.instant()
      const elapsed = now.since(initialAt, { largestUnit: 'hours', smallestUnit: 'seconds' })

      const recentMemory = episodicMemory?.flatMap((mem) => {
        const createdAt = Temporal.Instant.from(mem.created_at)
        const duration = now.since(createdAt, { largestUnit: 'hours', smallestUnit: 'seconds' })

        return [
          `### ${mem.title} (${durationFormat.format(duration)} ago)`,
          mem.summary,
        ]
      }).join('\n\n') ?? ''

      const content = prompt
        .replace('{recent_memory}', recentMemory)
        .replace('{time}', now.toLocaleString())
        .replace('{session_start_time}', initialAt.toLocaleString())
        .replace('{elapsed_time}', `${durationFormat.format(elapsed)} ago`)

      const budget = tokenBudget ?? DEFAULT_TOKEN_BUDGET
      const trimmedMessages = truncateMessages(messagesRef.current, budget)

      const { text } = await generateText({
        apiKey: env.OPENAI_API_KEY,
        baseURL: env.OPENAI_BASE_URL!,
        frequencyPenalty: 0.3,
        maxSteps: 10,
        messages: [
          { content, role: 'system' },
          ...trimmedMessages,
        ],
        model: env.OPENAI_CHAT_MODEL!,
        presencePenalty: 0.3,
        temperature: 0.85,
        tools,
        topP: 0.9,
      })

      if (text != null && text.trim().length !== 0)
        await pushMessage({ content: text, role: 'assistant' })
    },
  )

  const isLoading = isToolsLoading || isMemoryLoading || isMutating

  return {
    clear,
    error,
    isLoading,
    messages,
    send,
  }
}

