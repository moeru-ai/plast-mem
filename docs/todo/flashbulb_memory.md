# Flashbulb Memory (TODO)

## What is Flashbulb Memory?

In cognitive science, flashbulb memories are vivid, highly detailed snapshots of emotionally significant events — the kind of memory where you remember exactly where you were and what you were doing. They resist forgetting in a way ordinary memories do not.

For a cyber waifu, flashbulb memories are moments of high emotional significance that should **never fade**:
- First conversation
- User sharing something deeply personal
- A breakthrough moment or emotional turning point
- User explicitly saying "remember this"

## Design

Flashbulb Memory is not a new memory type — it is a **flag on Episodic Memory** that modifies FSRS behavior.

### Schema Change

Add a single field to [`EpisodicMemory`](../../crates/core/src/memory/episodic.rs):

```rust
pub struct EpisodicMemory {
    // ... existing fields ...
    pub is_flashbulb: bool,  // NEW
}
```

```sql
ALTER TABLE episodic_memory ADD COLUMN is_flashbulb BOOLEAN NOT NULL DEFAULT false;
```

### FSRS Behavior

| | Normal Episode | Flashbulb Episode |
|---|---|---|
| Retrievability | Decays over time via FSRS | Fixed at **1.0** |
| Memory Review | LLM evaluates relevance | **Skipped** |
| Stability/Difficulty | Updated by reviews | **Frozen** (never updated) |
| Retrieval ranking | Score × retrievability | Score × **1.0** (always full weight) |

In code, the change is minimal — in `retrieve()`, when computing the final score:

```rust
let retrievability = if memory.is_flashbulb {
    1.0
} else {
    compute_retrievability(memory.stability, memory.last_reviewed_at)
};
let final_score = rrf_score * retrievability;
```

And in the review job, skip flashbulb memories:

```rust
if memory.is_flashbulb {
    continue; // no review needed
}
```

### How Flashbulb Memories Are Created

Two paths:

1. **Automatic** — Episode surprise score exceeds a high threshold (e.g., `surprise >= 0.85`). The assumption: extreme surprise correlates with emotional significance.

2. **Explicit** — Future API endpoint or user command: "remember this forever." This is more reliable but requires UX design.

MVP should support at least automatic detection. Explicit marking can follow.

### Interaction with Semantic Memory

Flashbulb episodes are still processed by the Semantic Extraction Job — they contribute facts just like any other episode. The difference is only in FSRS behavior: the episode itself never fades, but the knowledge extracted from it is treated normally.

## Implementation Plan

- [ ] Add `is_flashbulb: bool` to `episodic_memory` table (migration)
- [ ] Update `EpisodicMemory` struct and entity
- [ ] Modify `retrieve()` — skip retrievability decay for flashbulb memories
- [ ] Modify review job — skip flashbulb memories
- [ ] Set `is_flashbulb = true` when `surprise >= 0.85` (FLASHBULB_SURPRISE_THRESHOLD) during episode creation
- [ ] Optional: API endpoint for explicit flashbulb marking

## What We Don't Do

- **No separate storage**: Flashbulb memories live in the same table as normal episodes.
- **No special retrieval path**: They go through the same hybrid search, just with retrievability pinned to 1.0.
- **No emotional classification**: We use surprise as a proxy for emotional significance, not a dedicated emotion detector.
