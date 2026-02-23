import type { Message } from '@xsai/shared-chat'

import { Box, Text, useInput } from 'ink'
import { useState } from 'react'

// Nord-inspired Color Palette
const COLORS = {
  accent: '#5E81AC', // Deep sea blue for selection
  border: '#4C566A', // Dark slate blue-gray
  muted: '#434C5E', // Deep gray for hints
  text: '#E5E9F0', // Light ice white
  title: '#81A1C1', // Muted ice blue
  warning: '#EBCB8B', // Amber
}

interface DebugHistoryPanelProps {
  history: HistoryItem[]
  onClose: () => void
}

interface HistoryItem {
  history: Message[]
  system_prompt: string
  timestamp: string
}

const HistoryRow = ({ isSelected, item }: { isSelected: boolean, item: HistoryItem }) => {
  const time = item.timestamp || 'Unknown Time'
  const promptPreview = item.system_prompt
    ? `${item.system_prompt.substring(0, 60).replace(/\n/g, ' ')}...`
    : 'No prompt content'

  const bgColor = isSelected ? COLORS.accent : undefined
  const textColor = isSelected ? '#ECEFF4' : COLORS.text
  const symbol = isSelected ? '› ' : '  '

  return (
    <Box
      backgroundColor={bgColor}
      paddingX={1}
    >
      <Text
        color={textColor}
        wrap="truncate-end"
      >
        {symbol}
        {time}
        {' '}
        -
        {' '}
        {promptPreview}
      </Text>
    </Box>
  )
}

const ConversationMsg = ({ msg }: { msg: Message }) => {
  const rawContent = msg.content
  const content = typeof rawContent === 'string'
    ? rawContent
    : JSON.stringify(rawContent)

  const roleAbbr = msg.role[0]?.toUpperCase() ?? 'U'
  const roleColor = msg.role === 'user' ? COLORS.accent : '#A3BE8C'
  const roleTag = `[${roleAbbr}] `
  const displayContent = content.length > 80 ? `${content.substring(0, 80)}...` : content

  return (
    <Box marginTop={0}>
      <Text bold color={roleColor}>
        {roleTag}
      </Text>
      <Text color={COLORS.text} wrap="truncate-end">
        {displayContent}
      </Text>
    </Box>
  )
}

const DetailsView = ({ item }: { item: HistoryItem }) => {
  const historyItems = (item.history || []).map((msg, i) => (
    <ConversationMsg key={`${msg.role}-${i}`} msg={msg} />
  ))

  return (
    <Box
      borderColor={COLORS.accent}
      borderStyle="single"
      flexDirection="column"
      marginBottom={1}
      marginTop={1}
      padding={1}
    >
      <Box justifyContent="space-between" marginBottom={1}>
        <Text bold color={COLORS.title} underline>
          Detailed Request Analysis
        </Text>
        <Text color={COLORS.muted}>Press Enter to close</Text>
      </Box>

      <Box marginBottom={1}>
        <Text color={COLORS.accent}>Timestamp: </Text>
        <Text color={COLORS.text}>{item.timestamp}</Text>
      </Box>

      <Box flexDirection="column" marginBottom={1}>
        <Text bold color={COLORS.title}>[System Prompt]</Text>
        <Box marginTop={0} paddingLeft={2}>
          <Text color={COLORS.text} italic>
            {item.system_prompt || 'Empty'}
          </Text>
        </Box>
      </Box>

      <Box flexDirection="column">
        <Text bold color={COLORS.title}>[Conversation Context]</Text>
        <Box flexDirection="column" paddingLeft={2}>
          {historyItems}
        </Box>
      </Box>
    </Box>
  )
}

export const DebugHistoryPanel = ({ history, onClose }: DebugHistoryPanelProps) => {
  const displayedHistory = [...history].reverse()
  const [displayedIndex, setDisplayedIndex] = useState(Math.max(0, displayedHistory.length - 1))
  const [isDetailExpanded, setIsDetailExpanded] = useState(false)

  useInput((input, key) => {
    if (key.escape) {
      onClose()
      return
    }

    if (isDetailExpanded) {
      if (key.return) {
        setIsDetailExpanded(false)
      }
      return
    }

    if (key.upArrow) {
      setDisplayedIndex(prev => Math.max(0, prev - 1))
    }

    if (key.downArrow) {
      setDisplayedIndex(prev => Math.min(displayedHistory.length - 1, prev + 1))
    }

    if (key.return && displayedHistory.length > 0) {
      setIsDetailExpanded(true)
    }
  })

  const selectedItem = displayedHistory[displayedIndex]

  return (
    <Box flexDirection="column" marginTop={1}>
      <Box
        borderColor={COLORS.border}
        borderStyle="round"
        flexDirection="column"
        paddingX={2}
        paddingY={0}
      >
        <Box marginBottom={1} marginTop={-1} paddingX={1}>
          <Text bold color={COLORS.title}>
            Debug History (Last
            {' '}
            {history.length}
            {' '}
            Requests)
          </Text>
        </Box>

        {history.length === 0
          ? (
              <Box paddingY={1}>
                <Text color={COLORS.muted} italic>
                  No requests recorded yet. Press Esc to exit.
                </Text>
              </Box>
            )
          : (
              <Box flexDirection="column" marginBottom={1}>
                {displayedHistory.map((item, index) => (
                  <HistoryRow
                    isSelected={index === displayedIndex}
                    item={item}
                    key={`${item.timestamp}-${item.system_prompt.substring(0, 10)}`}
                  />
                ))}
              </Box>
            )}

        {isDetailExpanded && selectedItem != null
          ? (
              <DetailsView
                item={selectedItem}
              />
            )
          : null}
      </Box>

      <Box marginTop={0} paddingX={1}>
        <Text color={COLORS.muted}>
          ↑/↓ select   ↵ view details   Esc close
        </Text>
      </Box>
    </Box>
  )
}
