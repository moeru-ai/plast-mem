/* eslint-disable @masknet/jsx-no-logical */
import { exit } from 'node:process'

import uuid from '@insel-null/uuid'
import TextInput from 'ink-text-input'

import { Box, Text } from 'ink'
import { useCallback, useMemo, useState } from 'react'

import { Header } from './components/header'
import { MessageBox } from './components/message'
import { useConversationId } from './hooks/use-dotenv-storage'
import { useHaru } from './hooks/use-haru'
import { useTerminalTitle } from './hooks/use-terminal-title'

const COMMANDS = [
  { cmd: '/model', desc: 'choose what model to use' },
  { cmd: '/clear', desc: 'clear working memory' },
  { cmd: '/reset', desc: 'reset Haru (dangeriously)' },
  { cmd: '/exit', desc: 'exit Haru' },
]

export const ChatApp = () => {
  useTerminalTitle('🌷 Haru')

  const [conversationId, setConversationId] = useConversationId()
  const { clear, isLoading, messages, send } = useHaru(conversationId)

  const [input, setInput] = useState('')
  const isCommand = useMemo(() => input.startsWith('/'), [input])
  const filteredCommands = useMemo(() => isCommand
    ? COMMANDS.filter(c => c.cmd.startsWith(input.toLowerCase()))
    : [], [input, isCommand])

  const handleSubmit = useCallback(async (value: string) => {
    if (!value.trim())
      return

    setInput('')

    if (value.startsWith('/')) {
      if (value === '/clear') {
        clear()
      }
      else if (value === '/exit') {
        exit(0)
      }
      else if (value === '/reset') {
        const newId = uuid.v7()
        setConversationId(newId)
      }
      return
    }

    await send(value)
  }, [clear, send, setConversationId])

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
                        <Box gap={2} key={item.cmd}>
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
