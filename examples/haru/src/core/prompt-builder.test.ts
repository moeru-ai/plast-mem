import type { EpisodicMemory } from 'plastmem'

import { Temporal } from 'temporal-polyfill'
import { describe, expect, it } from 'vitest'

import { buildSystemPrompt } from './prompt-builder'

describe('buildSystemPrompt', () => {
  const now = Temporal.Instant.from('2023-11-01T12:00:00Z')
  const initialAt = Temporal.Instant.from('2023-11-01T11:00:00Z')

  it('should build a complete prompt with recent memories', () => {
    const episodicMemory: EpisodicMemory[] = [
      {
        created_at: '2023-11-01T11:30:00Z',
        id: 'mem_1',
        importance: 0.8,
        last_accessed_at: '2023-11-01T11:30:00Z',
        summary: 'User was frustrated with a borrow checker error.',
        title: 'Rust Borrow Checker',
      },
      {
        created_at: '2023-10-31T10:00:00Z',
        id: 'mem_2',
        importance: 0.5,
        last_accessed_at: '2023-10-31T10:00:00Z',
        summary: 'User mentioned loving FastAPI and Pydantic.',
        title: 'Favorite Python Libraries',
      },
    ]

    const prompt = buildSystemPrompt({
      episodicMemory,
      initialAt,
      now,
    })

    // Check that placeholders are replaced
    expect(prompt).not.toContain('{time}')
    expect(prompt).not.toContain('{recent_memory}')
    expect(prompt).not.toContain('{examples}')

    // Snapshot test to ensure the overall structure remains consistent
    expect(prompt).toMatchSnapshot()
  })

  it('should handle cases with no recent memories', () => {
    const prompt = buildSystemPrompt({
      episodicMemory: [], // Empty array
      initialAt,
      now,
    })

    // The recent memory section should be empty
    const expectedAfterReplacement = `## Recent Memory



These are recent memories only.`
    expect(prompt).toContain(expectedAfterReplacement)

    // Snapshot test for the "empty" state
    expect(prompt).toMatchSnapshot()
  })

  it('should handle cases where episodicMemory is undefined', () => {
    const prompt = buildSystemPrompt({
      episodicMemory: undefined, // undefined
      initialAt,
      now,
    })

    // The recent memory section should be empty
    const expectedAfterReplacement = `## Recent Memory



These are recent memories only.`
    expect(prompt).toContain(expectedAfterReplacement)

    // Should be identical to the empty array case
    expect(prompt).toMatchSnapshot()
  })
})
