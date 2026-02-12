# Semantic Memory (TODO)

Semantic Memory is a knowledge distillation layer that extracts abstract facts from concrete episodic events. While Episodic Memory stores "what happened," Semantic Memory stores "what is true."

## Comparison

| Dimension | Episodic Memory | Semantic Memory |
|-----------|----------------|-----------------|
| Content | Specific events | Abstract knowledge |
| Example | "Yesterday we discussed Rust borrow checker" | "Rust borrow checker's core rule is..." |
| Trigger | Automatic segmentation | Extracted from high-surprise episodic memories |
| Mutability | Immutable (historical facts) | Mergeable and updatable (knowledge evolution) |
| Retrieval | Similarity + FSRS | Exact match + relationship traversal |
| Forgetting | FSRS retrievability decay | Superseded by new knowledge |

## Data Model

```rust
pub struct SemanticMemory {
    pub id: Uuid,

    /// Knowledge triple: entity - relation - target
    pub entity: String,
    pub relation: RelationType,
    pub target: String,

    /// Provenance tracking
    pub source_memories: Vec<Uuid>,
    pub confidence: f32,  // 0.0 ~ 1.0

    /// Metadata
    pub extracted_at: DateTime<Utc>,
    pub last_validated_at: DateTime<Utc>,
    pub access_count: i32,
}

pub enum RelationType {
    IsA,
    HasProperty,
    RelatedTo,
    PartOf,
    Causes,
    UsedFor,
    Contradicts,  // Critical for conflict detection
}
```

## Extraction Pipeline

### Trigger Condition

Semantic extraction runs as a batch job processing unprocessed EpisodicMemory entries. Only memories with `surprise > 0.3` are selected, as high-surprise events are more likely to contain novel knowledge.

### Workflow

```rust
pub async fn extract_from_episodic(
    episodic_id: Uuid,
    db: &DatabaseConnection,
) -> Result<Vec<SemanticMemory>, AppError> {
    let episodic = EpisodicMemory::get(episodic_id, db).await?;

    // LLM extracts knowledge triples
    let triples = llm::extract_knowledge_triples(&episodic.content).await?;

    for (entity, relation, target, conf) in triples {
        if let Some(existing) = SemanticMemory::find(&entity, &relation, db).await? {
            if existing.target == target {
                // Consistent: reinforce confidence
                existing.reinforce(conf).await?;
            } else {
                // Conflict: flag for resolution
                existing.flag_conflict(&target, episodic_id).await?;
            }
        } else {
            // New knowledge
            SemanticMemory::new(entity, relation, target, vec![episodic_id], conf)
                .save(db).await?;
        }
    }
}
```

### Knowledge Extraction Prompt

```
Extract knowledge triples from the following content.
Format: entity | relation | target | confidence(0-1)

Relations: IS_A, HAS_PROPERTY, RELATED_TO, PART_OF, CAUSES, PREVENTS

Example:
Input: "Rust's borrow checker prevents data races at compile time"
Output:
Rust borrow checker | IS_A | compile-time verification mechanism | 0.9
Rust borrow checker | PREVENTS | data races | 0.85
```

## Conflict Resolution

When new knowledge contradicts existing knowledge:

| Strategy | When to Use |
|----------|-------------|
| KeepBoth | Subjective knowledge where multiple views valid |
| HigherConfidence | Confidence differs significantly (>0.3 gap) |
| MoreRecent | Temporal knowledge that updates over time |
| HumanReview | Critical knowledge requiring verification |

Default: `HigherConfidence` with conflict logging.

## Unified Retrieval

```rust
pub async fn retrieve(query: &str, db: &DatabaseConnection) -> Result<RetrievalResult, AppError> {
    let query_type = classify_query(query);

    match query_type {
        QueryType::Factual => {
            // Prioritize semantic, use episodic for verification
            let semantic = SemanticMemory::search(query, db).await?;
            let supporting = EpisodicMemory::get_by_ids(&semantic.source_memories, db).await?;
            Ok(RetrievalResult::Factual { semantic, supporting })
        }
        QueryType::Event => {
            let episodic = EpisodicMemory::retrieve(query, limit, db).await?;
            Ok(RetrievalResult::Eventual(episodic))
        }
        QueryType::Mixed => {
            let semantic = SemanticMemory::search(query, db).await?;
            let expanded_query = expand_query(query, &semantic);
            let episodic = EpisodicMemory::retrieve(&expanded_query, limit, db).await?;
            Ok(RetrievalResult::Mixed { semantic, episodic })
        }
    }
}
```

## No FSRS for Semantic Memory

Semantic Memory does not use FSRS scheduling. Knowledge is not "forgotten" but **superseded**:

- New consistent knowledge: updates confidence
- New contradictory knowledge: triggers conflict resolution
- Unused knowledge: remains available (no decay)

Rationale: Factual truth does not decay; it is either correct, outdated, or refined.

## Database Schema

```sql
CREATE TYPE relation_type AS ENUM (
    'IsA', 'HasProperty', 'RelatedTo', 'PartOf',
    'Causes', 'UsedFor', 'Contradicts'
);

CREATE TABLE semantic_memory (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    entity TEXT NOT NULL,
    relation relation_type NOT NULL,
    target TEXT NOT NULL,
    source_episodic_ids UUID[] NOT NULL,
    confidence FLOAT NOT NULL CHECK (confidence >= 0 AND confidence <= 1),
    extracted_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    last_validated_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    access_count INTEGER DEFAULT 0,

    UNIQUE(entity, relation, target)
);

CREATE INDEX idx_semantic_entity ON semantic_memory(entity);
CREATE INDEX idx_semantic_triple ON semantic_memory(entity, relation);
```

## Implementation Phases

1. **Phase 1**: Add `surprise` field to EpisodicMemory
2. **Phase 2**: Add `BoundaryType` and `BoundaryContext` to EpisodicMemory
3. **Phase 3**: Implement SemanticMemory table and extraction job
4. **Phase 4**: Integrate unified retrieval with query classification

## Dependencies

- `docs/todo/surprise.md`: Surprise detection for extraction triggering
- `docs/todo/boundary_types.md`: Boundary context for source memory selection
