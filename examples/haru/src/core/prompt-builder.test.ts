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
        consolidated_at: null,
        conversation_id: 'conv_1',
        created_at: '2023-11-01T11:30:00Z',
        difficulty: 0.3,
        end_at: '2023-11-01T11:30:00Z',
        id: 'mem_1',
        last_reviewed_at: '2023-11-01T11:30:00Z',
        messages: [],
        stability: 1.0,
        start_at: '2023-11-01T11:00:00Z',
        summary: 'User was frustrated with a borrow checker error.',
        surprise: 0.8,
        title: 'Rust Borrow Checker',
      },
      {
        consolidated_at: null,
        conversation_id: 'conv_1',
        created_at: '2023-10-31T10:00:00Z',
        difficulty: 0.3,
        end_at: '2023-10-31T10:00:00Z',
        id: 'mem_2',
        last_reviewed_at: '2023-10-31T10:00:00Z',
        messages: [],
        stability: 1.0,
        start_at: '2023-10-31T09:00:00Z',
        summary: 'User mentioned loving FastAPI and Pydantic.',
        surprise: 0.5,
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
    expect(prompt).not.toContain('{semantic_context}')

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
