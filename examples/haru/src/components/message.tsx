import type { Message } from "@xsai/shared-chat";
import { Box, Text } from "ink";

export const MessageBox = ({ message }: { message: Message }) => {
  return (
    <Box backgroundColor={message.role === 'user' ? 'grey' : undefined} padding={1}>
      <Box marginRight={1}>
        <Text>{message.role === 'user' ? '›' : '•'}</Text>
      </Box>
      <Text>{message.content as string}</Text>
    </Box>
  )
}
