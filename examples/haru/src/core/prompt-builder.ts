import type { EpisodicMemory } from 'plastmem'

import { Temporal } from 'temporal-polyfill'

import { basePrompt, examples } from './prompt'

export interface SystemPromptArgs {
  episodicMemory?: EpisodicMemory[]
  initialAt: Temporal.Instant
  now: Temporal.Instant
  semanticContext?: string
}

/**
 * Applies a dictionary of values to a template string with placeholders.
 * @param template The template string, using {key} as placeholders.
 * @param data A Record<string, any> object (dictionary) to populate the template.
 * @returns The populated string.
 */
const applyTemplate = (template: string, data: Record<string, any>): string => {
  let result = template
  for (const [key, value] of Object.entries(data)) {
    // Use split().join() for safe and fast replacement of all occurrences
    // This avoids RegExp construction and potential ReDoS vulnerabilities
    const placeholder = `{${key}}`
    result = result.split(placeholder).join(String(value))
  }
  return result
}

/**
 * Formats a Temporal.Duration into a narrow string (e.g., "1h 30m").
 */
const formatDuration = (duration: { hours: number, minutes: number, seconds: number }): string => {
  const parts = []
  if (duration.hours > 0)
    parts.push(`${duration.hours}h`)
  if (duration.minutes > 0)
    parts.push(`${duration.minutes}m`)
  if (duration.seconds > 0 && parts.length === 0)
    parts.push(`${duration.seconds}s`)
  return parts.join(' ') || '0s'
}

/**
 * Builds the system prompt for Haru by injecting dynamic content into the base prompt.
 *
 * @param args - The dynamic arguments needed to build the prompt.
 * @param args.episodicMemory - The recent episodic memories to inject.
 * @param args.initialAt - The session start time.
 * @param args.now - The current time.
 * @returns The fully assembled system prompt string.
 */
export const buildSystemPrompt = ({
  episodicMemory,
  initialAt,
  now,
  semanticContext,
}: SystemPromptArgs): string => {
  const recentMemoryText
    = episodicMemory
      ?.map((mem) => {
        const createdAt = Temporal.Instant.from(mem.created_at)
        const duration = now.since(createdAt, {
          largestUnit: 'hours',
          smallestUnit: 'seconds',
        })

        return `### ${mem.title} (${formatDuration(duration)} ago)\n\n${mem.summary}`
      })
      .join('\n\n') ?? ''

  const elapsed = now.since(initialAt, {
    largestUnit: 'hours',
    smallestUnit: 'seconds',
  })

  // Create a dictionary of values to apply to the template
  const templateData = {
    elapsed_time: `${formatDuration(elapsed)} ago`,
    examples,
    recent_memory: recentMemoryText,
    semantic_context: semanticContext ?? '',
    session_start_time: initialAt.toLocaleString(),
    time: now.toLocaleString(),
  }

  // Apply the dictionary to the base prompt
  return applyTemplate(basePrompt, templateData)
}
