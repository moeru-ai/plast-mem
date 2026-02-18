# Semantic Memory

## What is Semantic Memory?

In cognitive science, Episodic Memory records *what happened* — concrete experiences tied to time and context. Semantic Memory stores *what I know* — knowledge, preferences, and facts distilled from many experiences.

Complementary Learning Systems (CLS) theory describes this:
- **Hippocampus** = Episodic Memory: rapid encoding of single experiences
- **Neocortex** = Semantic Memory: slow extraction of patterns across experiences

Plast Mem implements this via **delayed consolidation**: experiences initially exist only as episodes. Over time (or when highly surprising), they are replayed and consolidated into stable semantic facts.

### Value for Cyber Waifu

| Without Semantic Memory | With Semantic Memory |
|---|---|
| Must search episodes to know user preferences | Directly knows "he prefers dark themes" |
| Same fact scattered across 50 episodes | One fact record with provenance |
| Retrieval always returns episode fragments | Can directly answer factual questions |
| No awareness of relationship dynamics | Knows "we usually joke around" as a relational fact |

## Design: Delayed Consolidation (Offline Replay)

We moved from immediate per-episode extraction to a **batch consolidation** model. This better aligns with CLS theory and enables cross-episode pattern recognition.

### Consolidation Triggers

1.  **Threshold Trigger**: Consolidate when **3 unconsolidated episodes** accumulate.
2.  **Flashbulb Trigger**: Consolidate **immediately** if an episode has surprise ≥ 0.90.

### The Predict-Calibrate Loop

Consolidation is not just extraction; it is a belief update process.

1.  **Predict**: Before looking at new episodes, we retrieve existing semantic facts related to them. This represents "what we already believe."
2.  **Calibrate**: The LLM reviews the new episodes *in the context of* existing beliefs. It determines whether the new experiences:
    - Reveal **New** facts
    - **Reinforce** existing facts
    - **Update** existing facts (e.g., preference change)
    - **Invalidate** existing facts (e.g., contradiction)

### Fact Structure

```rust
pub struct SemanticMemory {
    pub id: Uuid,
    pub subject: String,       // "user", "we", "assistant"
    pub predicate: String,     // "likes", "lives_in"
    pub object: String,        // "Rust", "Tokyo"
    pub fact: String,          // Natural language: "User lives in Tokyo"
    pub source_episodic_ids: Vec<Uuid>,
    pub valid_at: DateTime<Utc>,
    pub invalid_at: Option<DateTime<Utc>>, // NULL = active
    /* ... embeddings ... */
}
```

## Data Flow

```
Event Segmentation -> Episode Created
       |
       v
Check Consolidation Triggers
       |
       +-> If Threshold < 3 AND Surprise < 0.90:
       |      Accumulate (Do nothing)
       |
       +-> If Threshold >= 3 OR Surprise >= 0.90:
              Trigger SemanticConsolidationJob
                       |
                       v
              1. Fetch unconsolidated episodes
              2. Fetch related existing facts (Context)
                       |
                       v
              3. LLM Consolidation (Predict-Calibrate)
                 Input: Existing Facts + New Episodes
                 Output: Actions [New, Reinforce, Update, Invalidate]
                       |
                       v
              4. Execute Actions
                 - New: Insert + Embed
                 - Reinforce: Update source_ids
                 - Update/Invalidate: Set invalid_at, Insert new
                       |
                       v
              5. Mark episodes as consolidated
```

## Retrieval

Semantic retrieval is **vector-only** (embeddings of the `fact` sentence).
- Filters for `invalid_at IS NULL` (only active beliefs).
- No FSRS decay (facts don't fade like episodes).
- Presented alongside episodic memories in retrieval results.

## Implementation Status

- [x] **Database**: `semantic_memory` table, `consolidated_at` on episodes.
- [x] **Entities**: Rust structs and SeaORM models.
- [x] **Consolidation Job**: Batch processing with accumulation logic.
- [x] **Core Logic**: `process_consolidation` pipeline with LLM integration.
- [x] **Retrieval**: Integrated into `retrieve_memory` API.

## Future Improvements (Phase 2)

- **Active Inquiry**: If consolidation reveals ambiguity, generate a question for the user.
- **Graph Queries**: Expose subject/predicate structure for complex reasoning.
- **Consistency Check**: Periodic background job to find and resolve latent contradictions.
