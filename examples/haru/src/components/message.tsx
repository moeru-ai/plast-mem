import type { Message } from '@xsai/shared-chat'

import { Box, Text } from 'ink'

export const MessageBox = ({ message }: { message: Message }) => {
  const backgroundColor = message.role === 'user' ? 'grey' : undefined
  const symbol = message.role === 'user' ? '›' : '•'

  return (
    <Box backgroundColor={backgroundColor} padding={1}>
      <Box marginRight={1}>
        <Text>{symbol}</Text>
      </Box>
      <Text>{message.content as string}</Text>
    </Box>
  )
}
