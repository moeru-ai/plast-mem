# Semantic Memory Extension (TODO)

Phase 2+ enhancements for the semantic memory system.

---

## 1. Semantic Note

### 1.1 Concept

A **Semantic Note** is an auto-generated summary entity that serves as a navigational hub for a specific subject. When a subject accumulates sufficient facts, the system synthesizes a coherent overview that acts as both a retrieval entry point and a compressed representation of long-term knowledge.

```
Before: 15 scattered facts about "user"
         ↓
Semantic Note: "User is a software engineer living in Tokyo..."
         ↓
Links to: [key_fact_1, key_fact_2, ...] + related notes
```

### 1.2 Schema

```rust
// plastmem_entities/src/semantic_note.rs
pub struct SemanticNote {
    pub id: Uuid,
    pub subject: String,              // e.g., "user", "we", "assistant"
    pub summary: String,              // Natural language synthesis
    pub key_facts: Vec<Uuid>,         // IDs of representative facts
    pub related_notes: Vec<Uuid>,     // Links to other SemanticNotes
    pub fact_count: i32,              // Facts at generation time
    pub embedding: PgVector,          // Summary embedding for retrieval
    pub generated_at: DateTime<Utc>,
    pub refreshed_at: DateTime<Utc>,
}

pub struct SemanticNoteConfig {
    pub min_facts: usize,             // Trigger threshold (default: 10)
    pub max_key_facts: usize,         // Cap on key_facts (default: 5)
    pub refresh_interval: Duration,   // Regeneration interval (default: 7 days)
}
```

### 1.3 Lifecycle

```
Fact added to subject
        │
        ▼
┌─────────────────────┐
│ Count active facts  │
└──────────┬──────────┘
           │
     ┌─────┴─────┐
     │           │
  < threshold  >= threshold
     │           │
     ▼           ▼
   Skip    ┌────────────────┐
           │ Note exists?   │
           └───────┬────────┘
                   │
            ┌──────┴──────┐
            │             │
           No           Yes
            │             │
            ▼             ▼
    ┌──────────────┐  ┌────────────────────┐
    │ Generate new │  │ Incremental check  │
    │ SemanticNote │  └───────┬────────────┘
    └──────────────┘          │
                        ┌─────┴─────┐
                        │           │
                  Minor change   Major change
                  (< 20% new     (≥ 20% new facts
                   facts, no      OR invalidation
                   invalidation)  occurred)
                        │           │
                        ▼           ▼
                      Skip    Regenerate Note
```

> [!NOTE]
> **Incremental check** avoids regenerating the entire note when only a few new facts arrive. Most new facts don't change a subject's overall profile. Full regeneration is triggered when fact count changes by ≥ 20% or when an existing fact is invalidated (indicating a meaningful belief change).

### 1.4 Generation Prompt

```markdown
You are synthesizing a Semantic Note for subject: "{subject}"

Input Facts (grouped by predicate):
```
## Personal Information
- lives_in: Tokyo (sources: 3)
- works_at: Cyberdyne (sources: 2)

## Preferences
- likes: Rust (sources: 5), TypeScript (sources: 2)
- dislikes: Java (sources: 1)

## Relationship Dynamics
- communicate_in_style: playful banter (sources: 4)
```

Generate:

1. **Summary** (3-5 sentences): A coherent paragraph capturing the essence.
   Prioritize: high-frequency facts > relationship dynamics > unique identifiers

2. **Key Facts** (max 5): IDs of the most representative facts.
   Selection criteria:
   - Facts with multiple sources
   - Unique identifiers (name, location)
   - Core relationship patterns

Output as JSON:
{
  "summary": "string",
  "key_fact_ids": ["uuid", ...]
}
```

### 1.5 Retrieval Integration

Semantic Notes participate in retrieval with elevated importance:

```rust
// crates/core/src/memory/retrieval.rs
pub async fn retrieve_memory(
    query: &str,
    db: &DatabaseConnection,
) -> Result<RetrievalResult> {
    // 1. Search Semantic Notes (higher weight, lower threshold)
    let notes = search_semantic_notes(query, threshold=0.75, limit=2, db).await?;

    // 2. Search individual facts
    let facts = search_semantic_memory(query, threshold=0.85, limit=10, db).await?;

    // 3. Expand notes: include their key_facts
    let expanded = expand_notes(&notes, &facts, db).await?;

    // 4. Deduplicate and rank
    let merged = merge_and_rerank(notes, expanded, facts);

    Ok(RetrievalResult {
        semantic_notes: notes,
        facts: merged,
        episodes: // ... episodic retrieval
    })
}
```

### 1.6 Cross-Subject Linking

Links between Semantic Notes are discovered automatically via shared fact objects:

```rust
// crates/core/src/memory/semantic_note.rs
pub fn discover_related_notes(
    subject: &str,
    facts: &[SemanticMemory],
    all_notes: &[SemanticNote],
) -> Vec<Uuid> {
    // Collect all objects from this subject's facts
    let objects: HashSet<&str> = facts.iter()
        .map(|f| f.object.as_str())
        .collect();

    // Find notes for other subjects that share any object
    all_notes.iter()
        .filter(|note| note.subject != subject)
        .filter(|note| {
            // Check if any of this note's key facts share an object
            note.key_facts_objects.iter().any(|obj| objects.contains(obj.as_str()))
        })
        .map(|note| note.id)
        .collect()
}
```

**Example**: "user likes Rust" + "assistant should use Rust examples" → `user` note ↔ `assistant` note linked through shared object "Rust".

### 1.7 Presentation Format

```markdown
## Semantic Overview
User is a software engineer who lives in Tokyo and works at Cyberdyne.
They have a strong preference for Rust and TypeScript, and our conversations
typically involve playful banter. [Key facts: 5] [Related: assistant]

## Known Facts
- User lives in Tokyo (sources: 3 conversations)
- User likes Rust (sources: 5 conversations)
...
```

---

## 2. Adaptive Surprise Threshold

### 2.1 Problem

Fixed thresholds (0.85, 0.90) assume uniform user behavior. A quiet user with
baseline surprise 0.2 will have most episodes flagged; an active user with
baseline 0.7 will rarely trigger deep extraction.

### 2.2 Design

Per-user surprise profile with sliding window statistics:

```rust
// plastmem_entities/src/surprise_profile.rs
pub struct SurpriseProfile {
    pub user_id: String,

    // Sliding window (last N episodes, default: 100)
    pub window_size: usize,
    pub scores: VecDeque<f32>,

    // Cached statistics
    pub mean: f32,
    pub std: f32,

    // Computed thresholds
    pub deep_extraction_threshold: f32,
    pub explanation_threshold: f32,

    pub updated_at: DateTime<Utc>,
}

impl SurpriseProfile {
    pub fn update(&mut self, new_score: f32) {
        self.scores.push_back(new_score);
        if self.scores.len() > self.window_size {
            self.scores.pop_front();
        }
        self.recompute_stats();
    }

    fn recompute_stats(&mut self) {
        let n = self.scores.len() as f32;
        self.mean = self.scores.iter().sum::<f32>() / n;

        let variance = self.scores.iter()
            .map(|s| (s - self.mean).powi(2))
            .sum::<f32>() / n;
        self.std = variance.sqrt();

        // Thresholds: mean + k*std, with floor values
        self.deep_extraction_threshold =
            (self.mean + 0.5 * self.std).max(0.70);
        self.explanation_threshold =
            (self.mean + 1.0 * self.std).max(0.80);
    }
}
```

### 2.3 Cold Start

```rust
impl SurpriseProfile {
    pub fn new(user_id: &str) -> Self {
        Self {
            user_id: user_id.to_string(),
            window_size: 100,
            scores: VecDeque::new(),
            mean: 0.5,
            std: 0.2,
            // Start with global defaults
            deep_extraction_threshold: 0.85,
            explanation_threshold: 0.90,
            updated_at: Utc::now(),
        }
    }
}
```

### 2.4 Integration

```rust
// crates/worker/src/jobs/semantic_extraction.rs
pub async fn process_episode(
    episode: &EpisodicMemory,
    db: &DatabaseConnection,
) -> Result<ExtractionOutput> {
    // Load or initialize profile
    let mut profile = SurpriseProfile::load(&episode.user_id, db).await?
        .unwrap_or_else(|| SurpriseProfile::new(&episode.user_id));

    // Use adaptive thresholds
    let config = if episode.surprise >= profile.explanation_threshold {
        ExtractionConfig::DeepWithExplanation
    } else if episode.surprise >= profile.deep_extraction_threshold {
        ExtractionConfig::Deep
    } else {
        ExtractionConfig::Standard
    };

    // Extract facts...
    let output = extract_facts(episode, config).await?;

    // Update profile with this episode's score
    profile.update(episode.surprise);
    profile.save(db).await?;

    Ok(output)
}
```

### 2.5 Behavior Examples

| User Type | History | Mean | Std | Deep Threshold | Effect |
|-----------|---------|------|-----|----------------|--------|
| Quiet | 0.1-0.3 | 0.20 | 0.05 | 0.23 | Minor deviations trigger deep extraction |
| Balanced | 0.4-0.6 | 0.50 | 0.10 | 0.55 | Moderate sensitivity |
| Chaotic | 0.6-0.9 | 0.75 | 0.15 | 0.83 | Only significant surprises flagged |

---

## 3. Inferred Relations

### 3.1 Scope

Lightweight inference rules that derive implicit knowledge without full knowledge graph complexity. Supports transitive and hierarchical patterns.

### 3.2 Schema

```rust
// plastmem_entities/src/inferred_relation.rs
pub struct InferredRelation {
    pub id: Uuid,
    pub source_fact_id: Uuid,         // Supporting direct fact
    pub inferred_fact: InferredFact,
    pub rule: InferenceRule,
    pub confidence: f32,              // < 1.0, inferred < direct
    pub expires_at: DateTime<Utc>,    // TTL for cache invalidation
}

pub struct InferredFact {
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub statement: String,            // Natural language
}

pub enum InferenceRule {
    // Location: (X, lives_in, Y) + (Y, is_in, Z) → (X, lives_in, Z)
    TransitiveLocation,

    // Category: (user, likes, X) + (X, is_a, Y) → (user, likes, Y) [weak]
    CategoryPreference,

    // Temporal: (A, happened_before, B) + (B, happened_before, C) → (A, happened_before, C)
    TemporalChain,

    // Complement: (user, likes, X) + (Y, is_opposite_of, X) → weak (user, dislikes, Y)
    ComplementaryPreference,
}
```

### 3.3 Inference Rules

```rust
// crates/core/src/inference/rules.rs

pub struct InferenceEngine;

impl InferenceEngine {
    /// Rule: Transitive Location
    /// (X, lives_in, Tokyo) + (Tokyo, is_in, Japan) → (X, lives_in, Japan)
    pub fn infer_locations(
        &self,
        facts: &[SemanticMemory],
    ) -> Vec<InferredRelation> {
        let mut results = vec![];

        // Index: object -> facts where object is subject
        let by_subject: HashMap<_, Vec<_>> = facts
            .iter()
            .filter(|f| f.predicate == "is_in")
            .map(|f| (f.subject.as_str(), f))
            .into_group_map();

        for fact in facts.iter().filter(|f| f.predicate == "lives_in") {
            if let Some(parents) = by_subject.get(fact.object.as_str()) {
                for parent in parents {
                    results.push(InferredRelation {
                        source_fact_id: fact.id,
                        inferred_fact: InferredFact {
                            subject: fact.subject.clone(),
                            predicate: "lives_in".to_string(),
                            object: parent.object.clone(),
                            statement: format!(
                                "{} lives in {}",
                                fact.subject, parent.object
                            ),
                        },
                        rule: InferenceRule::TransitiveLocation,
                        confidence: 0.85,
                        ..Default::default()
                    });
                }
            }
        }

        results
    }

    /// Rule: Category Preference (weak)
    /// (user, likes, Rust) + (Rust, is_a, PL) → (user, has_interest_in, PL) [0.6]
    pub fn infer_categories(
        &self,
        facts: &[SemanticMemory],
    ) -> Vec<InferredRelation> {
        // Similar implementation...
    }
}
```

### 3.4 Lazy Evaluation

Inferences are computed on-demand and cached with TTL:

```rust
// crates/core/src/inference/engine.rs
pub async fn get_facts_with_inferences(
    subject: &str,
    db: &DatabaseConnection,
) -> Result<Vec<EnrichedFact>> {
    // 1. Direct facts
    let direct = get_active_facts(subject, db).await?;

    // 2. Check cache
    let cached = get_cached_inferences(subject, db).await?;
    let needs_refresh = cached.first()
        .map(|c| c.expires_at < Utc::now())
        .unwrap_or(true);

    let inferred = if needs_refresh {
        // 3. Compute fresh inferences
        let engine = InferenceEngine;
        let mut all_inferred = vec![];

        all_inferred.extend(engine.infer_locations(&direct));
        all_inferred.extend(engine.infer_categories(&direct));

        // 4. Cache with TTL
        cache_inferences(subject, &all_inferred, ttl=Duration::hours(24), db).await?;
        all_inferred
    } else {
        cached
    };

    // 5. Merge and return
    Ok(merge_facts(direct, inferred))
}
```

### 3.5 Confidence Scoring

| Rule Type | Base Confidence | Notes |
|-----------|----------------|-------|
| Direct fact | 1.0 | From source_episodic_ids count |
| TransitiveLocation | 0.85 | Reliable for admin boundaries |
| TemporalChain | 0.75 | Each hop reduces confidence |
| CategoryPreference | 0.60 | Preferences don't always generalize |
| ComplementaryPreference | 0.50 | Highest uncertainty |

### 3.6 Presentation

Inferred facts are marked but included naturally:

```markdown
## Known Facts
- User lives in Tokyo (sources: 3 conversations)
- User lives in Japan (inferred: Tokyo is in Japan, confidence: 85%)
- User likes Rust (sources: 5 conversations)
- User has interest in programming languages (inferred: Rust is a PL, confidence: 60%)
```

---

## 4. Implementation Phasing

### Phase 2 Early: Semantic Note

1. Add `semantic_note` table migration
2. Create entity and CRUD operations
3. Implement generation trigger in `SemanticExtractionJob`
4. Implement incremental update check (≥ 20% new facts OR invalidation → regenerate)
5. Implement cross-subject linking via shared objects
6. Modify retrieval to include notes
7. Update presentation format

### Phase 2 Mid: Adaptive Threshold

1. Add `surprise_profile` table
2. Implement sliding window statistics
3. Integrate into extraction job
4. Add metrics/logging for threshold changes

### Phase 2 Late: Inferred Relations

1. Define inference rule trait
2. Implement location and category rules
3. Add lazy evaluation with caching
4. Integration test: verify inference quality

---

## 5. Open Questions

1. ~~**Semantic Note links**: Should notes link to each other ("user" ↔ "we")? How are these discovered?~~ → **Resolved**: Auto-discovered via shared fact objects (see §1.6).

2. **Threshold bounds**: Should adaptive thresholds have hard upper/lower bounds (e.g., never below 0.6, never above 0.95)?

3. **Inference expansion**: Which additional rules provide value without over-inferring? (e.g., "friend_of" transitivity has symmetry concerns)

4. **Cache invalidation**: When underlying facts change, how aggressively should we invalidate inference caches?

5. **Incremental vs full regeneration**: Is the 20% fact-change threshold the right trigger for Semantic Note regeneration? May need empirical tuning.

---

## 6. Related Documents

- [Semantic Memory](semantic_memory.md) - Core semantic memory design
- [architecture/fsrs.md](../architecture/fsrs.md) - Scheduling and review
- [CHANGE_GUIDE.md](../CHANGE_GUIDE.md) - Implementation patterns
