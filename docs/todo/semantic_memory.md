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

Batch consolidation model aligned with CLS theory enables cross-episode pattern recognition.

### Consolidation Triggers

1. **Threshold Trigger**: Consolidate when **3 unconsolidated episodes** accumulate.
2. **Flashbulb Trigger**: Consolidate **immediately** if an episode has surprise ≥ 0.85.

### The Predict-Calibrate Loop

Consolidation is a belief update process.

1. **Predict**: Retrieve existing semantic facts related to the new episodes. This represents "what we already believe."
2. **Calibrate**: The LLM reviews new episodes *in the context of* existing beliefs and determines whether they:
   - Reveal **New** facts
   - **Reinforce** existing facts
   - **Update** existing facts (e.g., preference change)
   - **Invalidate** existing facts (e.g., contradiction)

### Fact Structure

```rust
pub struct SemanticMemory {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub category: String,          // One of 8 categories (see below)
    pub fact: String,              // Natural language: "User lives in Tokyo"
    pub keywords: Vec<String>,     // ["Tokyo"] — for BM25 entity recall
    pub source_episodic_ids: Vec<Uuid>,
    pub valid_at: DateTime<Utc>,
    pub invalid_at: Option<DateTime<Utc>>, // NULL = active
    /* ... embedding ... */
}
```

### 8 Categories

| Category | What it captures |
|----------|-----------------|
| `identity` | Name, location, occupation, demographics |
| `preference` | Likes, dislikes, favorites |
| `interest` | Topics and hobbies |
| `personality` | Communication style, emotional tendencies |
| `relationship` | Dynamics, shared references, routines |
| `experience` | Skills, background, past events |
| `goal` | Plans and aspirations |
| `guideline` | How the assistant *should* behave |

## Data Flow

```
Event Segmentation -> Episode Created
       |
       v
Check Consolidation Triggers
       |
       +-> If Threshold < 3 AND Surprise < 0.85:
       |      Accumulate (Do nothing)
       |
       +-> If Threshold >= 3 OR Surprise >= 0.85:
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
                 - New: Insert + Embed (category prefix in embed input)
                 - Reinforce: Update source_ids
                 - Update/Invalidate: Set invalid_at, Insert new
                       |
                       v
              5. Mark episodes as consolidated
```

## Retrieval

Semantic retrieval uses **hybrid BM25 + vector search** (RRF fusion):

- BM25 on `search_text` generated column (`fact || ' ' || keywords`) — entity names in keywords boost BM25 recall
- Vector search on embedding (embed input: `"{category}: {fact} {keywords}"`)
- Optional `category` filter for targeted queries (e.g., `"guideline"` only)
- Filters for `invalid_at IS NULL` (only active beliefs)
- No FSRS decay (facts don't fade like episodes)

## Implementation Status

- [x] **Database**: `semantic_memory` table with `category`, `keywords`, `search_text` generated column
- [x] **Entities**: Rust structs and SeaORM models
- [x] **Consolidation Job**: Batch processing, 8-category LLM prompt, predict-calibrate loop
- [x] **Core Logic**: `SemanticMemory::retrieve()` with hybrid BM25+vector and category filter
- [x] **Retrieval**: Integrated into `retrieve_memory` and `context_pre_retrieve` APIs
- [x] **Migration**: `m20260228_01_refactor_semantic_memory` (SPO → category+keywords)

## Future Improvements (Phase 2)

- **Active Inquiry**: If consolidation reveals ambiguity, generate a question for the user.
- **Consistency Check**: Periodic background job to find and resolve latent contradictions.
- **Adaptive Surprise Threshold**: Per-conversation sliding window statistics for flashbulb detection.
