# Segmentation

Plast Mem first attempts rule-based matching and falls back to an LLM-based event segmenter.

## Rules

- If the number of messages is less than five, do not split.
- If there are thirty or more messages, split (boundary_hint = None, LLM determines type).
- If the latest message is more than fifteen minutes after the previous one, split (boundary_hint = TemporalGap).
- If the latest message is five characters or fewer, do not split.

This can reduce some LLM calls.

## Event Segmentation

It is based on [Event Segmentation Theory](https://en.wikipedia.org/wiki/Psychology_of_film#Segmentation) and invokes an LLM to analyze the conversation via structured output (`segment_events`).

The LLM returns:
- **action**: "create" or "skip" (when check=true; always "create" when check=false)
- **summary**: concise summary of the conversation
- **surprise**: prediction error score (0.0 ~ 1.0)
- **boundary_type**: "ContentShift", "GoalCompletion", or "PredictionError"

## Boundary Type Resolution

The final boundary type is determined by priority:
1. If rule-based detection set a boundary_hint (e.g. TemporalGap), use it
2. If surprise > 0.7, override to PredictionError
3. Otherwise, use the LLM's boundary_type

## Surprise-Based FSRS Boost

Surprising events get higher initial FSRS stability:

```
boosted_stability = initial_stability * (1.0 + surprise * 0.5)
```

This means surprise=1.0 gives 1.5x stability (slower decay), while surprise=0.0 gives normal decay.
