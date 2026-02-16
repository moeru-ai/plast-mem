# Semantic Memory (TODO)

Semantic Memory extracts abstract facts from concrete episodic events. While Episodic Memory stores "what happened," Semantic Memory stores "what is true."

## Comparison

| Dimension | Episodic Memory | Semantic Memory |
|-----------|----------------|-----------------|
| Content | Specific events | Abstract facts |
| Example | "Yesterday we discussed Rust borrow checker" | "User's primary language: Rust" |
| Trigger | Automatic segmentation | Extracted from high-surprise episodic memories |
| Mutability | Immutable (historical facts) | Updatable (facts change over time) |
| Retrieval | Similarity + FSRS | Exact match on entity + relation |
| Forgetting | FSRS retrievability decay | Superseded by new knowledge (no FSRS) |

## Data Model

```rust
pub struct SemanticMemory {
    pub id: Uuid,

    /// Knowledge triple: entity - relation - target
    pub entity: String,
    pub relation: RelationType,
    pub target: String,

    /// Provenance tracking
    pub source_episodic_ids: Vec<Uuid>,
    pub confidence: f32,  // 0.0 ~ 1.0

    /// Usage tracking for retrieval ranking
    pub access_count: i32,

    /// Metadata
    pub extracted_at: DateTime<Utc>,
    pub last_accessed_at: DateTime<Utc>,
}

pub enum RelationType {
    IsA,
    HasProperty,
    RelatedTo,
    PartOf,
    Causes,
    UsedFor,
    Prefers,       // User preference ("User PREFERS dark mode")
    Contradicts,   // For conflict detection
}
```

## Extraction Pipeline

### Trigger Condition

Semantic extraction runs as a background job after episode creation. Only episodes with `surprise > 0.3` are selected — high-surprise events are more likely to contain novel knowledge worth extracting.

### Workflow

```rust
pub async fn extract_from_episodic(
    episodic_id: Uuid,
    db: &DatabaseConnection,
) -> Result<Vec<SemanticMemory>, AppError> {
    let episodic = EpisodicMemory::get(episodic_id, db).await?;

    // LLM extracts knowledge triples (structured output)
    let triples = ai::extract_knowledge_triples(&episodic.summary, &episodic.messages).await?;

    for triple in triples {
        if let Some(existing) = SemanticMemory::find(&triple.entity, &triple.relation, db).await? {
            if existing.target == triple.target {
                // Consistent: reinforce confidence
                existing.reinforce(triple.confidence, episodic_id).await?;
            } else {
                // Conflict: resolve (typically keep newer or higher confidence)
                resolve_conflict(&existing, &triple, episodic_id, db).await?;
            }
        } else {
            // New knowledge
            SemanticMemory::create(triple, vec![episodic_id], db).await?;
        }
    }
}
```

### Knowledge Extraction Prompt

```
Extract knowledge triples from the following conversation episode.
Focus on persistent facts, preferences, and relationships — NOT transient events.

Format (structured output):
- entity: The subject (e.g., "User", "Rust", "User's job")
- relation: One of IS_A, HAS_PROPERTY, RELATED_TO, PART_OF, CAUSES, USED_FOR, PREFERS
- target: The object/value
- confidence: 0.0 - 1.0

Example:
Input: "I've been doing Python for 5 years but my new team is all Rust"
Output:
  { entity: "User", relation: "HAS_PROPERTY", target: "5 years Python experience", confidence: 0.9 }
  { entity: "User", relation: "PREFERS", target: "Rust (current team)", confidence: 0.7 }
```

## Conflict Resolution

When new knowledge contradicts existing knowledge (same entity + relation, different target):

| Strategy | When to Use |
|----------|-------------|
| KeepBoth | Subjective knowledge where multiple views valid |
| HigherConfidence | Confidence differs significantly (>0.3 gap) |
| MoreRecent | Temporal knowledge that updates over time (default) |
| HumanReview | Critical knowledge requiring verification |

Default: `MoreRecent` — new fact supersedes old fact. The old record is deleted or archived to audit log.

## Retrieval

Semantic memories participate in the unified retrieval pipeline alongside episodic memories.

### Query Classification

```rust
pub async fn retrieve(query: &str, db: &DatabaseConnection) -> Result<RetrievalResult, AppError> {
    let query_type = classify_query(query);

    match query_type {
        QueryType::Factual => {
            // Prioritize semantic, include supporting episodes
            let semantic = SemanticMemory::search(query, db).await?;
            let supporting = EpisodicMemory::get_by_ids(&semantic.source_episodic_ids, db).await?;
            Ok(RetrievalResult::Factual { semantic, supporting })
        }
        QueryType::Event => {
            // Pure episodic retrieval
            let episodic = EpisodicMemory::retrieve(query, limit, None, db).await?;
            Ok(RetrievalResult::Eventual(episodic))
        }
        QueryType::Mixed => {
            // Both: semantic for facts, episodic for context
            let semantic = SemanticMemory::search(query, db).await?;
            let expanded_query = expand_query(query, &semantic);
            let episodic = EpisodicMemory::retrieve(&expanded_query, limit, None, db).await?;
            Ok(RetrievalResult::Mixed { semantic, episodic })
        }
    }
}
```

### Search Methods

1. **Exact match**: `entity + relation` lookup for direct queries ("What language does user prefer?")
2. **Entity prefix match**: Simple prefix search on entity name for fuzzy matching

Results are ranked by `confidence` weighted by `access_count`. More frequently accessed facts surface first.

## No FSRS for Semantic Memory

Semantic Memory does not use FSRS scheduling. Knowledge is not "forgotten" — it is either current or **superseded**:

- New consistent knowledge → updates confidence, adds source episode
- New contradictory knowledge → triggers conflict resolution, old fact replaced
- Unused knowledge → remains available (no decay)
- Accessed knowledge → `access_count` incremented, `last_accessed_at` updated

Rationale: Factual truth does not decay. Prioritization uses simple access frequency — no complex activation formula needed.

## Database Schema

```sql
CREATE TYPE relation_type AS ENUM (
    'IsA', 'HasProperty', 'RelatedTo', 'PartOf',
    'Causes', 'UsedFor', 'Prefers', 'Contradicts'
);

CREATE TABLE semantic_memory (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    entity TEXT NOT NULL,
    relation relation_type NOT NULL,
    target TEXT NOT NULL,
    source_episodic_ids UUID[] NOT NULL,
    confidence FLOAT NOT NULL CHECK (confidence >= 0 AND confidence <= 1),
    access_count INTEGER DEFAULT 0,
    extracted_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    last_accessed_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),

    UNIQUE(entity, relation, target)
);

CREATE INDEX idx_semantic_entity ON semantic_memory(entity);
CREATE INDEX idx_semantic_triple ON semantic_memory(entity, relation);
```

## Implementation Phases

1. **Phase 1**: Implement SemanticMemory entity, migration, and extraction worker job
2. **Phase 2**: Integrate into unified retrieval with query classification

## Dependencies

- Episodic Memory `surprise` field (already implemented)
- `plastmem_ai` structured output for triple extraction

## See Also

- [Episodic Memory](../architecture/episodic_memory.md) — Source of semantic extractions
