import figlet from "figlet";
import { Box, Text } from "ink";
import { env } from "node:process";
import { version } from "../../package.json";

export const Header = () => (
  <Box flexDirection="row" gap={2} padding={1}>
    <Text>
      {figlet.textSync('Haru', {
        font: 'Catwalk'
      })}
    </Text>
    <Box flexDirection="column">
      <Text>Haru v{version}</Text>
      <Text>BaseURL: {env.OPENAI_BASE_URL}</Text>
      <Text>Model: {env.OPENAI_CHAT_MODEL}</Text>
    </Box>
  </Box>
)
