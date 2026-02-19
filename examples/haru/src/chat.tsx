import type { Message, UserMessage } from '@xsai/shared-chat'

import TextInput from 'ink-text-input'

import { Box, Text } from 'ink'
import { useCallback, useState } from 'react'

import { Header } from './components/header'
import { MessageBox } from './components/message'
import { useTerminalTitle } from './hooks/use-terminal-title'

export const ChatApp = () => {
  useTerminalTitle('🌷 Haru')

  const [input, setInput] = useState('')

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
          <Text>❯</Text>
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
        <Text dimColor>? for shortcuts · / for commands</Text>
      </Box>
    </Box>
  )
}
