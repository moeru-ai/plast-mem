/* eslint-disable @masknet/jsx-no-logical */
import type { Message, UserMessage } from '@xsai/shared-chat'

import TextInput from 'ink-text-input'

import { Box, Text } from 'ink'
import { useCallback, useMemo, useState } from 'react'

import { Header } from './components/header'
import { MessageBox } from './components/message'
import { useTerminalTitle } from './hooks/use-terminal-title'

const COMMANDS = [
  { cmd: '/model', desc: 'choose what model to use' },
  { cmd: '/reset', desc: 'reset Haru (dangeriously)' },
  { cmd: '/exit', desc: 'exit Haru' },
]

export const ChatApp = () => {
  useTerminalTitle('🌷 Haru')

  const [input, setInput] = useState('')
  const isCommand = useMemo(() => input.startsWith('/'), [input])
  const filteredCommands = useMemo(() => isCommand
    ? COMMANDS.filter(c => c.cmd.startsWith(input.toLowerCase()))
    : [], [input, isCommand])

  const [messages, setMessages] = useState<Message[]>([])

  const handleSubmit = useCallback((value: string) => {
    setMessages(prevMessages => [
      ...prevMessages,
      {
        content: value,
        role: 'user',
      } satisfies UserMessage,
      {
        content: 'pong',
        role: 'assistant',
      },
    ])
    setInput('')
  }, [])

  return (
    <Box flexDirection="column">
      <Header />
      {messages.map((message, index) => (
        // eslint-disable-next-line react/no-array-index-key
        <MessageBox key={`message ${index}`} message={message} />
      ))}

      <Box backgroundColor="grey" padding={1}>
        <Box marginRight={1}>
          <Text>›</Text>
        </Box>
        {/* TODO: fix grey placeholder */}
        <TextInput
          data-test-id="text-input"
          onChange={setInput}
          onSubmit={handleSubmit}
          placeholder="Write a message..."
          showCursor
          value={input}
        />
      </Box>

      <Box paddingX={1} paddingY={1}>
        {isCommand
          ? (
              <Box
                flexDirection="column"
              >
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
