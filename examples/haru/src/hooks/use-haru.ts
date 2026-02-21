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

import promptTemplate from '../docs/PROMPT.md?raw'

const durationFormat = new Intl.DurationFormat('en', { style: 'narrow' })

export const useHaru = (conversation_id: string) => {
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const initialAt = useMemo(() => Temporal.Now.instant(), [conversation_id])

  const messagesRef = useRef<Message[]>([])
  const [messages, setMessages] = useState<Message[]>([])

  const clear = useCallback(() => {
    messagesRef.current = []
    // eslint-disable-next-line react-hooks-extra/no-direct-set-state-in-use-effect
    setMessages([])
  }, [])

  useEffect(() => clear(), [clear, conversation_id])

  const { data: tools, isLoading: isToolsLoading } = useSWR(['haru/tools', conversation_id], async () => {
    const retrieveMemoryTool = await tool({
      description: 'Search long-term memory for relevant facts and past episodes',
      execute: async query => retrieveMemory({ body: { conversation_id, query } }).then(res => res.data),
      name: 'retrieve_memory',
      parameters: z.string().describe('Search query'),
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

      const content = promptTemplate
        .replace('{recent_memory}', recentMemory)
        .replace('{time}', now.toLocaleString())
        .replace('{session_start_time}', initialAt.toLocaleString())
        .replace('{elapsed_time}', `${durationFormat.format(elapsed)} ago`)

      const { text } = await generateText({
        apiKey: env.OPENAI_API_KEY,
        baseURL: env.OPENAI_BASE_URL!,
        maxSteps: 10,
        messages: [
          { content, role: 'system' },
          ...messagesRef.current,
        ],
        model: env.OPENAI_CHAT_MODEL!,
        tools,
      })

      if (text != null)
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
