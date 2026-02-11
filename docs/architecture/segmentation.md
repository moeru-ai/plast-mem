# Segmentation

Plast Mem first attempts rule-based matching and falls back to an LLM-based event segmenter.

## Rules

- If the number of messages is less than five, do not split.
- If there are thirty or more messages, split.
- If the latest message is more than fifteen minutes after the previous one, split.
- If the latest news is five characters or fewer, do not split.

This can reduce some LLM calls.

## EventSegmentation

It is based on [Event Segmentation Theory](https://en.wikipedia.org/wiki/Psychology_of_film#Segmentation) and invokes an LLM to determine whether segmentation is required.
