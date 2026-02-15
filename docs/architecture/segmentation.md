# Event Segmentation

Plast Mem implements an event segmentation system aligned with **Event Segmentation Theory (EST)** and the **Two-Step Alignment Principle** (from the Nemori paper). The dual-channel boundary detection design (topic shift + surprise) is inspired by [HiMem](https://arxiv.org/abs/2410.21385).

The system continuously monitors the conversation stream to detect event boundaries—moments where the "current event model" (what is happening now) no longer predicts the incoming information efficiently.

## Architecture

The segmentation process follows a layered filtering approach to minimize latency and LLM costs while maintaining high accuracy.

```mermaid
flowchart TB
  A[New Message] --> B{Rule Layer}
  B -->|"Too short / Trivial"| S[Skip]
  B -->|Buffer Full| F["ForceCreate (Drain All)"]
  B -->|Time Gap| T["TimeBoundary (Keep Last)"]
  B -->|Check Needed| DC["Dual-Channel Boundary Detection"]
  DC --> SC["Surprise Channel"]
  DC --> TC["Topic Channel"]
  SC -->|"sim < 0.35"| E2["Boundary (Direct)"]
  SC -->|"sim ≥ 0.35"| TC
  TC -->|"sim ≥ 0.5"| S2[Skip (Update Rolling Avg)]
  TC -->|"sim < 0.5"| L3["LLM Topic Shift Detection"]
  L3 -->|"confidence ≥ 0.7"| E["Boundary Detected (Keep Last)"]
  L3 -->|"confidence < 0.7"| U["Update Event Model, Continue"]
  F --> G["Episode Generation"]
  T --> G
  E --> G
  E2 --> G
  G --> H["Create EpisodicMemory"]
  H --> I["Init Next Event Context"]
```

## 1. Rule Layer

Fast, zero-cost checks to handle obvious cases:

| Rule | Condition | Action |
|------|-----------|--------|
| Buffer too small | messages < 3 | Skip |
| Buffer full | messages ≥ 50 | ForceCreate (drain all) |
| Time gap | > 15 minutes since last message | TimeBoundary (keep last) |
| Content too short | total chars < 100 | Skip |
| Message too short | latest message < 5 chars | Skip |

**Code**: `crates/core/src/message_queue/segmentation.rs`

## 2. Dual-Channel Boundary Detection

When the rule layer determines `NeedsBoundaryDetection`, the system runs two independent channels. Either channel triggering results in a boundary (OR relationship).

**Code**: `crates/core/src/message_queue/boundary.rs`

### Surprise Channel

Detects when incoming information diverges significantly from the current event model—a direct measure of prediction error.

- Computes `cosine_sim(event_model_embedding, new_message_embedding)`
- **sim < 0.35**: High prediction error → boundary triggered **directly** (no LLM needed)
- **sim ≥ 0.35**: No surprise boundary

The surprise signal (`1 - cosine_sim`) is also recorded on the created episode for FSRS stability boosting.

For `ForceCreate` and `TimeBoundary` paths, surprise is set to 0.0 (event model may be stale, calculation would be meaningless).

### Topic Channel

Detects gradual topic shifts through a two-stage process:

#### Stage 1: Embedding Pre-filter

- Computes `cosine_sim(last_embedding, new_message_embedding)`
- **sim ≥ 0.5**: Same topic. Update `last_embedding` via rolling average (alpha=0.2). No boundary.
- **sim < 0.5**: Potential shift. Proceed to LLM.

#### Stage 2: LLM Topic Shift Detection

The LLM evaluates multiple dimensions:
- **Topic Coherence**: Did the topic shift significantly?
- **Intent Change**: Did the speaker's goal change (e.g., discussion → decision)?
- **Temporal/Discourse Markers**: Are there phrases like "by the way", "anyway"?

**Input**:
- Current **Event Model** (description of "what is happening now")
- Recent conversation history

**Output**:
- `is_boundary`: Boolean
- `confidence`: 0.0 - 1.0
- `signals`: Multi-dimensional scores (topic_shift, intent_shift, temporal_marker)
- `updated_event_model`: Updated description of the event (if NOT a boundary)

If `is_boundary` is true AND `confidence` ≥ 0.7, a boundary is confirmed.

**Code**: `llm_topic_shift_detect()` in `crates/core/src/message_queue/boundary.rs`

## 3. Episode Generation

Once a segment is finalized (via Force, Time, or Boundary Detection), we generate a structured **Episodic Memory**.

**Input**: The segmented messages

**Output**:
- **Title**: A concise 5-15 word title
- **Summary**: A third-person narrative of the event

**Code**: `crates/core/src/memory/creation.rs`

## Event Model & Context Maintenance

- **Event Model**: A textual description of the *current* situation. It helps the LLM detect when the situation changes. It is reset after an episode is created.
- **Event Model Embedding**: Vector representation of the event model, used by the surprise channel. Updated whenever the LLM updates the event model. Reset after episode creation.
- **Last Embedding**: A vector representation of the current event's context.
  - Updated via rolling average during the event.
  - When a boundary occurs, the **Edge Message** (the message that triggered the boundary) is used to initialize the `last_embedding` for the *next* event, ensuring immediate context for the new segment.

## Edge Message Handling

For **Time Boundaries** and **LLM Boundaries**, the message that *triggered* the boundary (e.g., the "By the way..." message) is considered the **start of the next event**.

- The system drains `messages[0..N-1]` to create the memory.
- `messages[N]` (the edge message) remains in the buffer to start the new event context.

## Surprise-Based FSRS Boost

Surprising events result in stronger initial memories (higher stability):

```rust
boosted_stability = initial_stability * (1.0 + surprise * 0.5)
```

## Code Locations

| Component | Location |
|-----------|----------|
| Rule-based segmentation | `crates/core/src/message_queue/segmentation.rs` |
| Dual-channel boundary detection | `crates/core/src/message_queue/boundary.rs` |
| Episode generation + creation | `crates/core/src/memory/creation.rs` |
| Queue state management | `crates/core/src/message_queue/state.rs` |
| Pending reviews | `crates/core/src/message_queue/pending_reviews.rs` |
| Job scheduling (thin shell) | `crates/worker/src/jobs/event_segmentation.rs` |
