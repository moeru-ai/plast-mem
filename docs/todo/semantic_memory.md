# Semantic Memory (TODO)

## What is Semantic Memory?

In cognitive science, Episodic Memory records *what happened* — concrete experiences tied to time and context. Semantic Memory stores *what I know* — knowledge, preferences, and facts distilled from many experiences.

Complementary Learning Systems (CLS) theory describes this:
- **Hippocampus** = Episodic Memory: rapid encoding of single experiences
- **Neocortex** = Semantic Memory: slow extraction of patterns across experiences

Plast Mem already has Episodic Memory (hippocampus). Semantic Memory is its "neocortex."

### Value for Cyber Waifu

| Without Semantic Memory | With Semantic Memory |
|---|---|
| Must search episodes to know user preferences | Directly knows "he prefers dark themes" |
| Same fact scattered across 50 episodes | One fact record with provenance |
| Retrieval always returns episode fragments | Can directly answer factual questions |
| No awareness of relationship dynamics | Knows "we usually joke around" as a relational fact |

## Cognitive Science Foundations

### Predict-Calibrate Principle (from Nemori)

Knowledge is not passively extracted but actively learned through a predict-calibrate loop, aligning with the Free-Energy Principle — the brain learns by minimizing prediction error.

```
  New Episode arrives
       │
       ▼
  Use existing Semantic Memories
  to predict episode content (Predict)
       │
       ▼
  Compare prediction vs actual (Calibrate)
       │
       ├─ Correct → reinforce existing fact
       └─ Wrong   → extract new fact / fix old fact
```

### Gist Extraction (Schema Theory)

Memory consolidation naturally favors *gist* over *detail*. Episodes (details) are consolidated into semantic memories (gist). This happens implicitly: the LLM extracts lasting knowledge and discards transient states.

### Our Simplification

Nemori's full Predict-Calibrate is a two-step async pipeline. We simplify:

> **At episode creation time, a single LLM call extracts facts.**

In Phase 1 (MVP), the LLM extracts without seeing existing facts. In Phase 2, existing facts are provided as context for the full predict-calibrate loop.

## Design

### Fact: The Unit of Semantic Memory

```rust
pub struct SemanticMemory {
    pub id: Uuid,

    // ── Triple ──
    pub subject: String,       // "user", "user's cat", "we", "Tokyo"
    pub predicate: String,     // "likes", "lives_in", "communicate_in_style"
    pub object: String,        // "Rust", "Tokyo", "playful banter"

    // ── Natural language form ──
    pub fact: String,          // "User lives in Tokyo"

    // ── Provenance ──
    pub source_episodic_ids: Vec<Uuid>, // source episode IDs (length = implicit confidence)

    // ── Bitemporal ──
    pub valid_at: DateTime<Utc>,            // Utc::now() at creation
    pub invalid_at: Option<DateTime<Utc>>,  // Utc::now() when invalidated (NULL = active)

    // ── Indexing ──
    #[serde(skip)]
    pub embedding: PgVector,   // embedding of `fact`
    #[serde(skip)]
    pub created_at: DateTime<Utc>, // system timestamp, not exposed in API
}
```

> [!NOTE]
> No explicit `confidence` field. The length of `source_episodic_ids` serves as a natural confidence proxy — a fact mentioned in 5 episodes is more reliable than one mentioned in 1. A computed confidence score can be added in a later version if needed.

### Why Both Triple AND Natural Language Sentence?

The **triple** (subject, predicate, object) enables structured operations:
- Query all facts about `"user"`
- Find all `"likes"` relations
- Future graph extension: subjects/objects become nodes, facts become edges

The **`fact` sentence** enables semantic operations:
- Embedding-based similarity search and deduplication
- Better retrieval quality ("User moved from Beijing to Tokyo" is richer than `(user, lives_in, Tokyo)`)
- Human-readable display

### Subject Categories

Subjects and objects are free-form strings. For cyber waifu, three patterns are important:

| Pattern | Examples | Purpose |
|---|---|---|
| **User** | `"user"`, `"user's cat"`, `"user's mother"` | Personal facts, preferences |
| **Assistant** | `"assistant"` | Persona traits shaped by the user |
| **We** | `"we"` | Relational dynamics, shared context |

The **"we" subject** captures the relationship itself — critical for emotional companionship:

```
("we", "communicate_in_style", "playful banter")
("we", "have_shared_reference", "that time the code caught fire")
("we", "relationship_is", "close friends")
```

### Predicate Consistency

Predicates are stored as free-form `String`. Consistency is achieved through **prompt guidance only** (no runtime normalization in MVP).

The extraction prompt provides recommended predicates:

```
Recommended predicates (use these when applicable, create new ones if needed):
- likes, dislikes, prefers
- lives_in, works_at, age_is, name_is
- is_interested_in, has_experience_with, knows_about
- communicate_in_style, relationship_is, has_shared_reference, has_routine
```

This is sufficient because:
- The same LLM tends to produce consistent output within a prompt
- Occasional duplicates ("likes" vs "enjoys") don't break retrieval (embedding similarity catches them)
- Runtime canonicalization can be added in a later phase if fragmentation becomes a real problem

### Bitemporal Model

| Field | Meaning | Value |
|---|---|---|
| `valid_at` | When we learned this fact | `Utc::now()` at creation |
| `invalid_at` | When we learned it was no longer true | `Utc::now()` when invalidated, `NULL` = active |

We do **not** ask the LLM to infer real-world timestamps ("last summer" → specific date). Both timestamps are simply `Utc::now()` at the moment we create or invalidate the fact.

**Active facts**: `invalid_at IS NULL`

**Example — residence change**:

```
Episode 1:  "I live in Beijing"
  → INSERT ("user", "lives_in", "Beijing")  valid_at: 2025-01-01, invalid_at: NULL

Episode 10: "I moved to Tokyo"
  → Phase 2: LLM detects conflict, sets invalid_at on Beijing fact
  → ("user", "lives_in", "Beijing")  valid_at: 2025-01-01, invalid_at: 2025-06-15
  → INSERT ("user", "lives_in", "Tokyo")  valid_at: 2025-06-15, invalid_at: NULL
```

In MVP (Phase 1), both facts simply coexist. `invalid_at` is only set in Phase 2 when LLM-based conflict detection is implemented.

### Deduplication and Conflict Resolution

#### Phase 1 (MVP): Embedding-Based Dedupe Only

```rust
async fn upsert_fact(extracted: &ExtractedFact, embedding: PgVector, source_episode_id: Uuid, db: &DatabaseConnection) {
    // 1. Find highly similar existing facts (strict threshold, up to 5)
    let similar = find_similar_facts(&embedding, 0.95, db).await?;

    // 2. If any similar facts found, merge into the most similar one
    if let Some(existing) = similar.first() {
        // True duplicate: merge source_episodic_ids (idempotent)
        append_source_episodic_ids(
            existing.id,
            &existing.source_episodic_ids,
            &[source_episode_id],
            db
        ).await?;
        return;
    }

    // 3. No match → insert as new fact
    // Even if it might contradict an existing fact (MVP accepts this)
    insert_fact(extracted, embedding, db).await?;
}
```

**Why 0.95?** Strict enough to only merge true duplicates ("User likes Rust" ≈ "user likes Rust"), without merging distinct facts ("likes Rust" vs "likes TypeScript" ≈ 0.85). At this threshold, facts with different objects ("name is Bob" vs "name is Alice") have similarity well below 0.95, so no object check is needed.

**MVP accepts contradictions** — "lives in Beijing" and "lives in Tokyo" can coexist. This is safe: better to preserve noisy signal than to silently delete valid facts with wrong heuristics.

#### Phase 2: LLM-Based Conflict Detection

When extracting facts, retrieve related existing facts as LLM context. The LLM determines whether new information invalidates an existing fact:

```
For each extracted fact, determine its relationship to existing facts:
- "new": No existing fact covers this.
- "reinforce": An existing fact says the same thing. Include its ID.
- "invalidate": An existing fact is no longer true. Include its ID.

Important: Multiple values for the same predicate can coexist
(e.g., liking multiple things). Only mark as "invalidate" when the
new information genuinely replaces the old (e.g., changing residence).
```

When a fact is invalidated: `UPDATE semantic_memory SET invalid_at = now() WHERE id = $1`.

### Data Flow: Episode → Facts (Surprise-Aware)

The extraction job adapts its behavior based on the episode's surprise score. This replaces a standalone Surprise Response system — surprise handling is folded into the extraction pipeline as a single LLM call.

```
 Event Segmentation creates Episode
              │
              ▼
     Semantic Extraction Job
              │
              ├─ 0. Extraction gate (skip low-information episodes)
              │     └─ Skip if: content.len() < 50 AND surprise < 0.3
              │
              ├─ 1. Check episode surprise score
              │     ├─ surprise < 0.85: standard extraction
              │     └─ surprise ≥ 0.85: deep extraction (more thorough prompt)
              │
              ├─ 2. LLM: extract facts from episode
              │     Input: episode summary + messages + surprise level
              │     Output: SemanticExtractionOutput
              │       └─ facts: Vec<ExtractedFact>
              │
              ├─ 3. Batch embed all extracted facts (single API call)
              │
              └─ 4. For each extracted fact:
                    ├─ Search for similar existing facts (cosine > 0.95)
                    ├─ Match found  → merge source_episodic_ids
                    └─ No match    → insert new fact
```

> [!NOTE]
> **Extraction gating** skips episodes that are too short and unsurprising to contain extractable facts (e.g., "好的", "收到"). This avoids unnecessary LLM calls — estimated 30-50% savings depending on conversation style.

**Why not a separate Surprise Response Job?** The "surprise response" actions (deeper extraction, explanation, belief updates) all happen within the same LLM context as fact extraction. A separate job would require a second LLM call to analyze the same episode, with largely redundant output. Folding it into `SemanticExtractionJob` is both cheaper and more coherent.

### LLM Extraction Interface

```rust
#[derive(Debug, Deserialize, JsonSchema)]
struct SemanticExtractionOutput {
    pub facts: Vec<ExtractedFact>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ExtractedFact {
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub fact: String,  // natural language sentence
}
```

**System prompt guidelines**:

```
Extract lasting knowledge from this conversation segment.

Categories to extract:
1. Facts about the user (preferences, personal info, relationships)
2. Facts about the relationship ("we" subject)
3. Behavioral rules for the assistant:
   - Communication preferences the user has expressed
   - Topics to avoid or emphasize
   - Interaction patterns and rituals
   - Conditional behavior (when X happens, do Y)

Rules:
1. Only extract long-term facts. Ignore transient states ("I'm hungry now" is NOT a fact).
2. Use subject-predicate-object format.
3. Include a natural language `fact` sentence for each triple.
4. Preferences, habits, personal info, relationships, and significant events are good candidates.
5. For behavioral rules, use subject = "assistant".

Predicate taxonomy (use these when applicable; use "other" for novel predicates):

  Personal: likes, dislikes, prefers, lives_in, works_at, age_is, name_is
  Knowledge: is_interested_in, has_experience_with, knows_about
  Relational: communicate_in_style, relationship_is, has_shared_reference, has_routine
  Behavioral: should, should_not, should_when_[context], responds_to_[trigger]_with
```

> [!NOTE]
> Grouping predicates into a taxonomy in the prompt reduces fragmentation (LLMs produce more consistent output when given categorical structure). This is a prompt-only convention — no runtime enforcement.

> [!NOTE]
> For high-surprise episodes (≥ 0.85), the prompt is extended with:
> ```
> This episode had a surprise score of {surprise}/1.0.
> Extract facts more thoroughly — pay attention to novel or unexpected information.
> ```
> Note: `surprise_explanation` was considered but removed in Phase 1 to keep the API simple.

### Procedural Memory via Semantic Facts

Procedural rules ("how to behave") are stored as **semantic facts with `subject = "assistant"`** and behavioral predicates. No separate table or extraction pipeline.

```
("assistant", "should", "use Rust examples when explaining code")
("assistant", "should_not", "mention user's ex")
("assistant", "should_when_user_upset", "be gentle and use shorter messages")
("assistant", "responds_to_oyasumi_with", "おやすみ、いい夢見てね 🌙")
("we", "have_routine", "Monday morning check-in about the weekend")
```

The boundary between "what we do" (semantic) and "how I should act" (procedural) is naturally fuzzy. Both are extracted by the same prompt and retrieved together. Separation only happens at presentation time.

### Retrieval

Semantic memories are returned **separately from episodic memories** in the existing `retrieve_memory` API, with procedural rules (behavioral guidelines) filtered into their own section:

```markdown
## Known Facts
- User likes Rust (sources: 3 conversations)
- User likes TypeScript (sources: 1 conversation)
- User's cat is named Mochi (sources: 2 conversations)
- We usually communicate with playful banter (sources: 4 conversations)

## Behavioral Guidelines
- When user is upset, be gentle and brief (sources: 1 conversation)
- Always use Rust examples when explaining code (sources: 2 conversations)

## Episodic Memories
## Memory 1 [rank: 1, score: 0.85]
...
```

Retrieval: **vector-only search** on the `fact` field. Only active facts (`invalid_at IS NULL`) are returned. No BM25 — semantic facts are short sentences (typically < 10 words) where TF-IDF scoring adds no discriminative value; embedding similarity is sufficient. No FSRS re-ranking — facts don't decay.

Presentation-time separation: facts where `subject = "assistant"` with procedural predicates (`should`, `should_not`, `should_when_*`, `responds_to_*_with`) are displayed under "Behavioral Guidelines". All other facts go under "Known Facts".

#### API Integration

No new endpoints. Extend the existing `retrieve_memory` handlers:

- **`/api/v0/retrieve_memory`** (markdown): Add `## Known Facts` and `## Behavioral Guidelines` sections before episodic memories in `format_tool_result()`
- **`/api/v0/retrieve_memory/raw`** (JSON): Extend response struct with `facts: Vec<SemanticMemoryResult>` alongside `memories`

Response structure:
```rust
pub struct RetrieveMemoryRawResponse {
    /// Semantic memories (known facts + behavioral guidelines)
    pub facts: Vec<SemanticMemoryResult>,
    /// Episodic memories with scores
    pub memories: Vec<RetrieveMemoryRawResult>,
}

pub struct SemanticMemoryResult {
    #[serde(flatten)]
    pub memory: SemanticMemory,
    /// Cosine similarity score
    pub score: f64,
}
```

This follows the principle of least surprise — callers get richer results from the same API.

### Database Schema

```sql
CREATE TABLE semantic_memory (
    id                   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    subject              TEXT NOT NULL,
    predicate            TEXT NOT NULL,
    object               TEXT NOT NULL,
    fact                 TEXT NOT NULL,
    source_episodic_ids  UUID[] NOT NULL DEFAULT '{}',
    valid_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
    invalid_at           TIMESTAMPTZ,
    embedding            vector(1024) NOT NULL,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Vector search on fact embedding
CREATE INDEX idx_semantic_memory_embedding ON semantic_memory
    USING hnsw (embedding vector_cosine_ops);

-- Active facts for a subject
CREATE INDEX idx_semantic_memory_active_subject ON semantic_memory (subject)
    WHERE invalid_at IS NULL;
```

## Implementation Plan

### Phase 1: MVP — Extract, Dedupe, Retrieve

- [x] `semantic_memory` table migration
- [x] `plastmem_entities::semantic_memory` entity
- [x] `plastmem_core::memory::semantic.rs` — `SemanticMemory` struct, CRUD, embedding dedupe
- [x] `SemanticExtractionJob` — triggered after episode creation
  - [x] Extraction gating: skip low-information episodes (short content + low surprise)
  - [x] Batch embedding: embed all extracted facts in a single API call
- [x] LLM extraction prompt (with predicate taxonomy + procedural category) + `generate_object()` call
- [x] Surprise-aware extraction: deeper prompt for surprise ≥ 0.85
- [x] `SemanticMemory::retrieve()` — vector-only search, filter `invalid_at IS NULL`
- [x] Modify `retrieve_memory` API: add `## Known Facts` + `## Behavioral Guidelines` sections
- [x] Update tool result format (presentation-time filter on `subject = "assistant"`)

### Phase 2: Predict-Calibrate + Conflict Resolution (incorporates Surprise Response)

- [ ] **Full predict-calibrate loop** for high-surprise episodes:
  - [ ] Load top-K related existing facts as context
  - [ ] LLM predicts episode content based on existing knowledge
  - [ ] Compare prediction vs actual → prediction gap drives extraction aggressiveness
- [ ] Extend `ExtractedFact` with `action` field ("new" / "reinforce" / "invalidate")
- [ ] LLM-based conflict detection (sets `invalid_at` on contradicted facts)
- [ ] For high-surprise episodes: LLM identifies which existing beliefs are challenged → `invalidate`
- [ ] Optional: predicate canonicalization via embedding similarity
- [ ] Optional: computed confidence score from `source_episodic_ids`

## Scenario Walkthrough

### A. Repeated mention (dedupe works)

```
Episode 1: "I like Rust"  → extract (user, likes, Rust)
Episode 5: "I like Rust"  → extract (user, likes, Rust)
                                 ↓
                     embedding similarity ~0.98
                                 ↓
                  merge source_episodic_ids = [ep1, ep5]
```

### B. Additive preferences (correctly preserved)

```
Episode 1: "I like Rust"        → (user, likes, Rust)
Episode 3: "I like TypeScript"  → (user, likes, TypeScript)
                                       ↓
                           embedding similarity ~0.85 (< 0.95)
                                       ↓
                        both facts coexist ✓
```

### C. Actual conflict (safe in MVP, resolved in Phase 2)

```
Episode 1:  "I live in Beijing"  → (user, lives_in, Beijing)
Episode 10: "I moved to Tokyo"  → (user, lives_in, Tokyo)
                                       ↓
                           embedding similarity ~0.80 (< 0.95)
                                       ↓
             MVP:     both coexist (safe, no data loss)
             Phase 2: LLM detects conflict → invalidate Beijing
```

### D. Correction (embedding distance prevents wrong merge)

```
Episode 1: "My name is Bob"    → (user, name_is, Bob)
Episode 3: "Sorry, my name is actually Alice"
                               → (user, name_is, Alice)
                                       ↓
                           embedding similarity ~0.88 (< 0.95)
                           different names = different embeddings
                                       ↓
             Both coexist (not merged)
             Phase 2: LLM detects correction → invalidate Bob
```

## Open Questions

1. **Dedupe threshold**: 0.95 is a starting point. Needs empirical validation — too low risks merging distinct facts, too high risks fragmentation.
2. **Extraction gate calibration**: The `content.len() < 50 AND surprise < 0.3` gate is conservative. Monitor false-negative rate (episodes that should have been extracted but were skipped).
3. **Surprise threshold calibration**: Should the 0.85/0.90 thresholds be adaptive based on the user's baseline surprise distribution?
4. **Predict-calibrate cost**: The full predict-calibrate loop adds one LLM call. Should it run for all episodes or only high-surprise ones?

## References

- [Nemori](https://arxiv.org/abs/2508.03341) — Predict-Calibrate principle, Free-Energy Principle
- [EDC Framework](https://aclanthology.org/2024.findings-naacl.7/) — Extract, Define, Canonicalize
- [A-MEM](https://arxiv.org/abs/2502.12110) — Zettelkasten-inspired agentic memory
- [Complementary Learning Systems](https://en.wikipedia.org/wiki/Complementary_learning_systems) — Hippocampus ↔ Neocortex
- [Active Inference](https://doi.org/10.1162/neco_a_00912) — Friston et al. (2017), process theory for free-energy minimization

## What We Don't Do

- **No knowledge graph engine**: Free-form triples stored in Postgres. Subjects/objects can become graph nodes in the future.
- **No BM25 for facts**: Facts are short sentences where TF-IDF adds no value. Vector-only search is sufficient and simpler.
- **No FSRS for facts**: Semantic knowledge doesn't follow forgetting curves.
- **No predicate enum**: Prompt-level taxonomy guidance only. Runtime canonicalization deferred.
- **No confidence formula**: `source_episodic_ids.len()` is sufficient for MVP.
- **No LLM conflict detection in MVP**: Embedding dedupe only. Contradictions are safe to coexist temporarily.
- **No separate procedural memory table**: Procedural rules reuse semantic memory with `subject = "assistant"` convention.
- **No separate Surprise Response system**: Surprise-aware behavior is folded into `SemanticExtractionJob` (surprise → deeper extraction + explanation), not a standalone job.
- **No follow-up question generation**: Could be added later if a consumer exists.
- **No self-reflection system**: Out of scope — no defined consumer or storage for reflections.
