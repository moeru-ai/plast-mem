import type { Message, UserMessage } from '@xsai/shared-chat';
import { Box, Text } from 'ink';
import TextInput from 'ink-text-input';
import { useCallback, useState } from 'react';
import { Header } from './components/header';
import { MessageBox } from './components/message';
import { useTerminalTitle } from './hooks/use-terminal-title';

export const ChatApp = () => {
  useTerminalTitle('🌷 Haru')

  const [input, setInput] = useState('');

  const [messages, setMessages] = useState<Message[]>([]);

  const handleSubmit = useCallback((value: string) => {
    setMessages(prevMessages => [
      ...prevMessages,
      {
        role: 'user',
        content: value,
      } satisfies UserMessage,
    ]);
    setInput('');
  }, [])

  return (
    <Box flexDirection="column">
      <Header />
      {messages.map((message, index) => (
        <MessageBox key={`message ${index}`} message={message} />
      ))}

      <Box backgroundColor='grey' padding={1}>
        <Box marginRight={1}>
          <Text>❯</Text>
        </Box>
        {/* TODO: fix grey placeholder */}
        <TextInput
          showCursor
          value={input}
          onChange={setInput}
          onSubmit={handleSubmit}
          placeholder='Write a message...'
        />
      </Box>

      <Box paddingY={1} paddingX={1}>
        <Text dimColor>? for shortcuts · / for commands</Text>
      </Box>
    </Box>
  );
}
