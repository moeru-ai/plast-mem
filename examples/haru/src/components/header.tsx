import { env } from 'node:process'

import figlet from 'figlet'

import { Box, Text } from 'ink'

import { version } from '../../package.json'

export const Header = () => (
  <Box flexDirection="row" gap={2} padding={1}>
    <Text color="blue">
      {figlet.textSync('Haru', {
        font: 'Catwalk',
      })}
    </Text>
    <Box alignSelf="flex-end" flexDirection="column" paddingBottom={1}>
      <Text>
        Haru v
        {version}
      </Text>
      <Text>
        <Text dimColor>model:</Text>
        {' '}
        {env.OPENAI_CHAT_MODEL}
        {' '}
        <Text dimColor>/model to change</Text>
      </Text>
      <Text>
        <Text dimColor>baseURL:</Text>
        {' '}
        {env.OPENAI_BASE_URL}
      </Text>
    </Box>
  </Box>
)
