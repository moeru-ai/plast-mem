# Event Segmentation

Plast Mem implements an event segmentation system aligned with **Event Segmentation Theory (EST)** and the **Two-Step Alignment Principle** (from the Nemori paper).

The system continuously monitors the conversation stream to detect event boundaries—moments where the "current event model" (what is happening now) no longer predicts the incoming information efficiently.

## Architecture

The segmentation process follows a layered filtering approach to minimize latency and LLM costs while maintaining high accuracy.

```mermaid
flowchart TB
  A[New Message] --> B{Rule Layer}
  B -->|"Too short / Trivial"| S[Skip]
  B -->|Buffer Full| F["ForceCreate (Drain All)"]
  B -->|Time Gap| T["TimeBoundary (Keep Last)"]
  B -->|Check Needed| L2["Embedding Similarity Layer"]
  L2 -->|"sim ≥ Threshold"| S2[Skip (Update Rolling Avg)]
  L2 -->|"sim < Threshold"| L3["LLM Boundary Detection"]
  L3 -->|"confidence ≥ Threshold"| E["Boundary Detected (Keep Last)"]
  L3 -->|"confidence < Threshold"| U["Update Event Model, Continue"]
  F --> G["LLM Episode Generation"]
  T --> G
  E --> G
  G --> H["Create EpisodicMemory"]
  H --> I["Init Next Event Context"]
```

## 1. Rule Layer

Fast, zero-cost checks to handle obvious cases:

- **Buffer Full**: If messages exceed 30, force a segment.
- **Time Gap**: If the time trace between messages exceeds 15 minutes, trigger a boundary. The triggering message is considered the *start* of the new event.
- **Minimum Content**: If total characters in buffer < 100, do not segment.

## 2. Embedding Similarity Layer

Low-cost semantic continuity check.

- Computes cosine similarity between the **Last Embedding** (rolling average of the current event's context) and the **New Message Embedding**.
- **Similarity ≥ 0.5**: The new message is semantically consistent with the current event. No boundary.
  - *Action*: Update `last_embedding` using a weighted moving average (alpha=0.2) to adapt to gradual topic evolution without drift.
- **Similarity < 0.5**: Potential boundary detected. Proceed to LLM check.

## 3. LLM Boundary Detection (Step 1)

If the embedding layer signals a potential shift, we invoke the LLM to verify if a meaningful **Event Boundary** has occurred.

The LLM evaluates multiple dimensions:
- **Topic Coherence**: Did the topic shift significantly?
- **Intent Change**: Did the speaker's goal change (e.g., discussion → decision)?
- **Temporal/Discourse Markers**: Are there phrases like "by the way", "anyway"?

**Input**:
- Current **Event Model** (description of "what is happening now").
- Recent conversation history.

**Output**:
- **is_boundary**: Boolean.
- **confidence**: 0.0 - 1.0.
- **updated_event_model**: Updated description of the event (if NOT a boundary).

If `is_boundary` is true AND `confidence` ≥ 0.7, a boundary is confirmed.

## 4. Episode Generation (Step 2)

Once a segment is finalized (via Force, Time, or LLM Boundary), we generate a structured **Episodic Memory**.

**Input**:
- The segmented messages.

**Output**:
- **Title**: A concise 5-15 word title.
- **Summary**: A third-person narrative of the event.
- **Surprise**: A 0.0 - 1.0 score representing information gain.

## Event Model & Context Maintenance

- **Event Model**: A textual description of the *current* situation. It helps the LLM detect when the situation changes. It is reset after an episode is created.
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
