# Boundary Types (TODO)

Boundary types categorize why an event segmentation occurred, enabling context-aware retrieval weighting based on Event Segmentation Theory.

## Type Definitions

```rust
pub enum BoundaryType {
    /// Time interval exceeds threshold (e.g., 15+ minutes)
    TemporalGap(f64),  // hours elapsed

    /// Topic or subject matter changed
    ContentShift,

    /// Task, goal, or intention completed
    GoalCompletion,

    /// Unexpected event, prediction error detected
    PredictionError,
}

pub struct BoundaryContext {
    pub boundary_type: BoundaryType,
    pub strength: f32,        // 0.0 ~ 1.0
    pub surprise: f32,         // Associated surprise score
}
```

## Detection Methods

| Boundary Type | Detection Method | Trigger Condition |
|--------------|------------------|-------------------|
| `TemporalGap` | Rule-based | Message timestamp delta > 15 minutes |
| `ContentShift` | LLM-based | Topic analysis shows discontinuity |
| `GoalCompletion` | LLM-based | Explicit completion markers or outcome achieved |
| `PredictionError` | Surprise threshold | `surprise > 0.7` in event analysis |

## Retrieval Boost

Boundary types influence retrieval scores through dynamic boost factors:

```rust
impl BoundaryContext {
    pub fn retrieval_boost(&self) -> f64 {
        match self.boundary_type {
            BoundaryType::PredictionError => {
                // Highest boost: unexpected events carry high-value signals
                1.3 + 0.2 * self.surprise
            }
            BoundaryType::GoalCompletion => {
                // Elevated boost: completion states summarize outcomes
                1.2
            }
            BoundaryType::ContentShift => {
                // Neutral: significance depends on content matching
                1.0
            }
            BoundaryType::TemporalGap(hours) => {
                // Reduced boost: longer gaps imply less continuity
                0.9 + 0.1 / (1.0 + hours.ln())
            }
        }
    }
}
```

### Applied in Retrieval

```rust
let final_score = rrf_score
    * retrievability as f64
    * boundary_boost
    * recency_factor;
```

## Rationale

| Boundary Type | Boost | Reasoning |
|--------------|-------|-----------|
| `PredictionError` | 1.3-1.5 | Unexpected events contain maximum learning value per EST |
| `GoalCompletion` | 1.2 | Outcome states provide closure and are often sought retrospectively |
| `ContentShift` | 1.0 | Neutral weight; relevance determined by content similarity |
| `TemporalGap` | 0.85-0.9 | Time discontinuity suggests weaker contextual relevance |

## Database Schema

```sql
CREATE TYPE boundary_type AS ENUM (
    'TemporalGap',
    'ContentShift',
    'GoalCompletion',
    'PredictionError'
);

ALTER TABLE episodic_memory ADD COLUMN
    boundary_type boundary_type NOT NULL DEFAULT 'ContentShift';

ALTER TABLE episodic_memory ADD COLUMN
    boundary_strength FLOAT NOT NULL DEFAULT 0.5;

-- Index for boundary-aware retrieval
CREATE INDEX idx_episodic_boundary
    ON episodic_memory(boundary_type, boundary_strength);
```

## Context Sensitivity

Boost values are static in the base implementation. Advanced usage may adjust based on query context:

| Query Pattern | Adjusted Boost Behavior |
|--------------|------------------------|
| "How did I solve..." | Elevate `GoalCompletion` |
| "What went wrong..." | Elevate `PredictionError` |
| "Previously we..." | Elevate `TemporalGap` (seek older context) |

This requires query intent classification, which can be added as a future enhancement.
