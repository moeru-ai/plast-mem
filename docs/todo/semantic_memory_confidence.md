# Semantic Memory Confidence (TODO)

## Overview

Add FSRS-based confidence tracking to semantic memory. Uses `stability` as confidence score with `elapsed_days=0` (no time decay). `Again` rating immediately invalidates facts; other ratings update confidence.

---

## Phase 1: Database Schema

- [ ] **Migration: Add confidence fields to semantic_memory table**
  - `confidence: DOUBLE` (maps to FSRS stability)
  - `difficulty: DOUBLE` (FSRS difficulty, 1.0-10.0)
  - `review_count: INTEGER` (total review times)
  - `consecutive_again: INTEGER` (for preventing false invalidation)
  - `last_reviewed_at: TIMESTAMPTZ`

- [ ] **Update `crates/entities/src/semantic_memory.rs`**
  - Add new fields to `Model` struct
  - Update `ActiveModel` if needed

---

## Phase 2: Core Logic

- [ ] **Create `crates/core/src/semantic_confidence.rs`**
  - Define `ConfidenceLevel` enum: `Unreliable`, `Weak`, `Moderate`, `Strong`, `VeryStrong`
  - `confidence_level(stability: f64) -> ConfidenceLevel` function
    - Thresholds: `<1.0`, `1.0-3.0`, `3.0-10.0`, `10.0-30.0`, `>30.0`
  - Initial confidence calculation based on source_episodes count:
    - 1 source: 2.0 (Good baseline)
    - 2 sources: 4.0
    - 3-5 sources: 6.0
    - 6+ sources: 8.0
  - Initial difficulty based on category:
    - `preference`, `goal`: 6.0 (volatile)
    - `identity`: 4.0 (stable)
    - others: 5.0

---

## Phase 3: Review Job

- [ ] **Create `crates/worker/src/jobs/semantic_review.rs`**
  - Define `SemanticReviewJob` struct:
    ```rust
    pub struct SemanticReviewJob {
        pub fact_ids: Vec<Uuid>,
        pub trigger: ReviewTrigger,  // ContradictionDetected, Retrieved, Scheduled
        pub context: ReviewContext,  // Episodes/facts for LLM context
        pub reviewed_at: DateTime<Utc>,
    }
    ```
  - LLM prompt for fact validity review (different from episodic relevance):
    - "again": Fact is contradicted by context → invalidate
    - "hard": Partially inconsistent or needs updating
    - "good": Consistent with context, still valid
    - "easy": Strongly supported by context
  - FSRS update logic:
    ```rust
    let days_elapsed = 0;  // Key: no time decay
    let next_states = fsrs.next_states(current_state, 0.9, 0)?;
    ```
  - Rating handling:
    - `Again` + `consecutive_again >= 2` → set `invalid_at`
    - `Again` + `consecutive_again < 2` → `confidence *= 0.3`, increment counter
    - `Hard` → `confidence *= 0.8`
    - `Good` → use `next_states.good.memory.stability`
    - `Easy` → use `next_states.easy.memory.stability`

---

## Phase 4: Integration Points

### Consolidation Trigger
- [ ] **Modify `crates/worker/src/jobs/predict_calibrate.rs`**
  - When LLM returns `update` or `invalidate` action:
    - Enqueue `SemanticReviewJob` for the old fact (if it exists)
  - When inserting new fact:
    - Use initial confidence/difficulty calculation
  - After successful reinforcement:
    - Optionally boost confidence slightly (or rely on review flow)

### Retrieval Flow
- [ ] **Modify `crates/core/src/memory/semantic.rs`**
  - `retrieve()` function:
    - Keep pure RRF ordering (NO confidence multiplication)
    - Optional: filter `ConfidenceLevel::Unreliable` if RRF < 0.9
    - Return `(SemanticMemory, f64, ConfidenceLevel)` tuples

- [ ] **Update `crates/core/src/memory/retrieval.rs`**
  - `format_tool_result()` for semantic facts:
    - Add confidence indicator: `✓`, `✓✓`, `?`, `??` (or empty for Moderate)
    - Example: `- [preference] User likes coffee [?]` (Weak confidence)

### API Layer
- [ ] **Update retrieval endpoints** (`crates/server/src/api/retrieve_memory.rs`)
  - Ensure raw endpoint returns confidence values
  - Markdown endpoint includes confidence indicators

---

## Phase 5: Periodic Review (Optional v2)

- [ ] **Create scheduled job for low-confidence facts**
  - Query: `confidence < 5.0 AND review_count < 3`
  - Enqueue batch review job
  - Run daily/weekly

---

## Phase 6: Testing

- [ ] **Unit tests**
  - `confidence_level()` mapping
  - Initial confidence calculation
  - FSRS update with `days_elapsed=0`

- [ ] **Integration tests**
  - End-to-end: consolidation → review → confidence update
  - Again → invalidate flow
  - Retrieval with confidence indicators

- [ ] **Edge cases**
  - Very high stability (>100) handling
  - Multiple consecutive Again ratings
  - Review of already-invalidated facts (should skip)

---

## Key Design Decisions (Reference)

| Aspect | Decision |
|--------|----------|
| **Time decay** | `elapsed_days = 0`, no time-based decay |
| **Again handling** | `consecutive_again >= 2` for actual invalidation |
| **Retrieval sorting** | Pure RRF, confidence only for filtering/annotation |
| **Confidence display** | 5-level discrete (Unreliable/Weak/Moderate/Strong/VeryStrong) |
| **Trigger** | Consolidation contradictions + retrieved usage |

---

## Open Questions

1. Should `guideline` category have different confidence handling? (Maybe harder to invalidate)
2. Should we store full review history or just counters?
3. Should retrieval trigger review job directly, or batch pending reviews?
