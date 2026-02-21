/* eslint-disable @masknet/jsx-no-logical */
import type { Message, Tool, UserMessage } from '@xsai/shared-chat'

import { randomUUID } from 'node:crypto'
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { homedir } from 'node:os'
import { join } from 'node:path'
import { env, exit } from 'node:process'

import TextInput from 'ink-text-input'

import { generateText } from '@xsai/generate-text'
import { Box, Text } from 'ink'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'

import promptTemplate from './docs/PROMPT.md?raw'

import { Header } from './components/header'
import { MessageBox } from './components/message'
import { useTerminalTitle } from './hooks/use-terminal-title'
import { addMessage, recentMemory, retrieveMemory } from './plastmem'

const COMMANDS = [
  { cmd: '/model', desc: 'choose what model to use' },
  { cmd: '/clear', desc: 'clear working memory' },
  { cmd: '/reset', desc: 'reset Haru (dangeriously)' },
  { cmd: '/exit', desc: 'exit Haru' },
]

const buildSystemPrompt = (recentMemoryText: string, sessionStart: Date): string => {
  const now = new Date()
  const elapsedMs = now.getTime() - sessionStart.getTime()
  const elapsedMin = Math.floor(elapsedMs / 60000)
  const elapsed = elapsedMin < 1 ? 'just now' : `${elapsedMin}m ago`
  return (promptTemplate as string)
    .replace('{recentMemory()}', recentMemoryText)
    .replace('{time}', now.toLocaleString())
    .replace('{session_start_time}', sessionStart.toLocaleString())
    .replace('{elapsed_time}', elapsed)
}

const loadConversationId = (): string => {
  const dir = join(homedir(), '.config', 'haru')
  const file = join(dir, 'id')
  try {
    return readFileSync(file, 'utf-8').trim()
  }
  catch {
    mkdirSync(dir, { recursive: true })
    const id = randomUUID()
    writeFileSync(file, id)
    return id
  }
}

export const ChatApp = () => {
  useTerminalTitle('🌷 Haru')

  const [input, setInput] = useState('')
  const isCommand = useMemo(() => input.startsWith('/'), [input])
  const filteredCommands = useMemo(() => isCommand
    ? COMMANDS.filter(c => c.cmd.startsWith(input.toLowerCase()))
    : [], [input, isCommand])

  const [messages, setMessages] = useState<Message[]>([])
  const [isLoading, setIsLoading] = useState(false)

  const conversationIdRef = useRef<string>('')
  const systemPromptRef = useRef<string>('')
  const sessionStartRef = useRef<Date>(new Date())

  useEffect(() => {
    const conversationId = loadConversationId()
    conversationIdRef.current = conversationId

    recentMemory(conversationId)
      .then((mem) => {
        systemPromptRef.current = buildSystemPrompt(mem, sessionStartRef.current)
      })
      .catch(() => {
        systemPromptRef.current = buildSystemPrompt('', sessionStartRef.current)
      })
  }, [])

  const handleSubmit = useCallback(async (value: string) => {
    if (!value.trim())
      return

    setInput('')

    if (value.startsWith('/')) {
      if (value === '/clear') {
        setMessages([])
      }
      else if (value === '/exit') {
        exit(0)
      }
      return
    }

    const conversationId = conversationIdRef.current

    const userMsg: UserMessage = { content: value, role: 'user' }
    setMessages(prev => [...prev, userMsg])

    addMessage(conversationId, 'user', value).catch(() => {})

    setIsLoading(true)

    const retrieveTool: Tool = {
      execute: async (input) => {
        const { query } = input as { query: string }
        return retrieveMemory(conversationId, query)
      },
      function: {
        description: 'Search long-term memory for relevant facts and past episodes',
        name: 'retrieve_memory',
        parameters: {
          properties: { query: { description: 'Search query', type: 'string' } },
          required: ['query'],
          type: 'object',
        },
      },
      type: 'function',
    }

    try {
      const history = messages.filter(m => m.role === 'user' || m.role === 'assistant')
      const result = await generateText({
        apiKey: env.OPENAI_API_KEY,
        baseURL: env.OPENAI_BASE_URL!,
        maxSteps: 5,
        messages: [
          { content: systemPromptRef.current, role: 'system' },
          ...history,
          userMsg,
        ],
        model: env.OPENAI_CHAT_MODEL!,
        tools: [retrieveTool],
      })

      const text = result.text ?? ''
      setMessages(prev => [...prev, { content: text, role: 'assistant' }])
      addMessage(conversationId, 'assistant', text).catch(() => {})
    }
    catch (err) {
      setMessages(prev => [...prev, { content: `error: ${String(err)}`, role: 'assistant' }])
    }
    finally {
      setIsLoading(false)
    }
  }, [messages])

  return (
    <Box flexDirection="column">
      <Header />
      {messages.map((message, index) => (
        // eslint-disable-next-line react/no-array-index-key
        <MessageBox key={`message ${index}`} message={message} />
      ))}

      {isLoading && (
        <Box padding={1}>
          <Text dimColor>...</Text>
        </Box>
      )}

      <Box backgroundColor="grey" padding={1}>
        <Box marginRight={1}>
          <Text>›</Text>
        </Box>
        <TextInput
          data-test-id="text-input"
          onChange={setInput}
          onSubmit={value => void handleSubmit(value)}
          placeholder="Write a message..."
          showCursor
          value={input}
        />
      </Box>

      <Box paddingX={1} paddingY={1}>
        {isCommand
          ? (
              <Box flexDirection="column">
                {filteredCommands.length > 0
                  ? (
                      filteredCommands.map((item, i) => (
                        <Box gap={2} key={i}>
                          <Text bold={i === 0} color={i === 0 ? 'blue' : undefined}>{item.cmd}</Text>
                          <Text bold={i === 0} color={i === 0 ? 'blue' : undefined} dimColor={i !== 0}>{item.desc}</Text>
                        </Box>
                      ))
                    )
                  : (
                      <Text dimColor>no matches</Text>
                    )}
              </Box>
            )
          : <Text dimColor>/ for commands</Text>}
      </Box>
    </Box>
  )
}
