import type { AssistantMessage, Message, UserMessage } from '@xsai/shared-chat'
import type { AddMessageMessage } from 'plastmem'

import { env } from 'node:process'

import useSWR from 'swr'
import useSWRMutation from 'swr/mutation'
import z from 'zod'

import { generateText } from '@xsai/generate-text'
import { tool } from '@xsai/tool'
import { addMessage, contextPreRetrieve, recentMemoryRaw, retrieveMemory } from 'plastmem'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Temporal } from 'temporal-polyfill'

import { buildSystemPrompt } from '../core/prompt-builder'

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
  const [requestHistory, setRequestHistory] = useState<object[]>([])

  const isFirstMountRef = useRef(true)

  const clear = useCallback(() => {
    messagesRef.current = []
    setMessages([])
    setRequestHistory([])
  }, [])

  useEffect(() => {
    if (isFirstMountRef.current) {
      isFirstMountRef.current = false
      return
    }
    clear()
  }, [clear, conversation_id])

  const { data: tokenBudget } = useSWR('haru/tokenBudget', async () => {
    try {
      const res = await fetch(`${env.OPENAI_BASE_URL}/models`, {
        headers: { Authorization: `Bearer ${env.OPENAI_API_KEY}` },
      })
      const json = await res.json() as { data: { context_length?: number, id: string }[] }
      const model = json.data.find(m => m.id === env.OPENAI_CHAT_MODEL)
      if (model?.context_length)
        return Math.floor(model.context_length * CONTEXT_WINDOW_RATIO)
    }
    catch (error) { console.error('Failed to fetch model context length, falling back to default.', error) }
    return DEFAULT_TOKEN_BUDGET
  }, { revalidateOnFocus: false })

  const { data: tools, isLoading: isToolsLoading } = useSWR(
    ['haru/tools', conversation_id],
    async () => {
      const retrieveMemoryTool = await tool({
        description: 'Search long-term memory for relevant facts and past episodes',
        execute: async ({ query }) =>
          retrieveMemory({ body: { conversation_id, query } }).then(
            res => res.data ?? String(res.error),
          ),
        name: 'retrieve_memory',
        parameters: z.object({
          query: z.string().describe('Search query'),
        }),
      })
      return [retrieveMemoryTool]
    },
    { revalidateOnFocus: false },
  )

  const { data: episodicMemory, isLoading: isMemoryLoading } = useSWR(
    ['haru/recentMemory', conversation_id],
    async () =>
      recentMemoryRaw({ body: { conversation_id } }).then(res => res.data),
  )

  const pushMessage = useCallback(
    async (message: AssistantMessage | UserMessage) => {
      messagesRef.current = [...messagesRef.current, message]
      setMessages(prev => [...prev, message])
      await addMessage({
        body: { conversation_id, message: message as AddMessageMessage },
      })
    },
    [conversation_id],
  )

  const { error, isMutating, trigger: send } = useSWRMutation<
    void,
    Error,
    [string, string],
    string
  >(
    ['haru/send', conversation_id],
    async (_, { arg: input }) => {
      if (input.trim().length === 0)
        return

      await pushMessage({ content: input, role: 'user' })

      const now = Temporal.Now.instant()

      const [semanticContext] = await Promise.allSettled([
        contextPreRetrieve({ body: { conversation_id, query: input } }).then(res => res.data ?? ''),
      ])

      const content = buildSystemPrompt({
        episodicMemory,
        initialAt,
        now,
        semanticContext: semanticContext.status === 'fulfilled' ? semanticContext.value : '',
      })

      const debugPayload = {
        history: messagesRef.current,
        system_prompt: content,
        timestamp: now.toLocaleString(),
      }
      setRequestHistory(prev => [debugPayload, ...prev].slice(0, 5))

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
    requestHistory,
    send,
  }
}
