# Event Segmentation v2 LoCoMo Experiment Log

This note summarizes the LoCoMo experiments done during the `Event Segmentation v2` refactor on 6 April 2026.

It is not a full design doc. The goal is to record what we changed, what actually improved the score, and what was clearly a bad direction.

## Scope

These experiments were run after the breaking `span_v2` refactor landed:

- `conversation_message` + `segmentation_state` + `episode_span`
- span-first segmentation pipeline
- larger episodic spans
- gated `PredictCalibrate`
- provisional episodic projection

Most experiments below were run on `conv-47`, with additional inspection on `conv-42` and `conv-44`.

## Baseline References

Reference points:

- Old baseline before v2 comparison point: 2026-04-03 10:19 run
- Earlier best v2 run: 2026-04-06 08:22 run
- Current best v2 run after rolling back failed title experiments: 2026-04-06 14:08 run

Key numbers:

| run | overall F1 | NemoriF1 | LLM | multi-hop | temporal | open-domain | single-hop |
| :--- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 2026-04-03 10:19 baseline | 49.78% | 47.34% | 66.00% | 24.93% | 26.76% | 20.82% | 69.73% |
| 2026-04-06 08:22 best v2 | 51.06% | 47.34% | 65.33% | 26.38% | 24.34% | 20.82% | 72.69% |
| 2026-04-06 14:08 current best v2 | 52.03% | 49.21% | 66.00% | 25.22% | 24.90% | 20.82% | 74.49% |

Takeaway:

- v2 can beat the earlier baseline overall.
- the current best v2 run is now above both the earlier baseline and the previous best v2 checkpoint.
- The remaining weak category is still `temporal`.
- `multi-hop` and `single-hop` improved after the segmentation and consolidation cleanup.

## What Helped

### 1. Larger episodic spans plus gated `PredictCalibrate`

This was the highest-value change.

Initial v2 segmentation was severely over-segmenting `conv-47`:

- `689` messages
- `498` `episode_span`
- average `1.38` messages per span

After tightening same-session boundaries, suppressing dense candidate clusters, and gating `PredictCalibrate`, `conv-47` moved to a much healthier shape:

- roughly `31-32` `episode_span`
- average about `21-22` messages per span
- `PredictCalibrateJob` count dropped from hundreds to roughly `30+`
- singleton noise was mostly removed

This was the turning point that made v2 competitive again.

### 2. Absorbing trailing singleton before a strong temporal gap

One concrete low-risk fix helped:

- previously there was a bad `80-80` singleton before a strong gap
- merging that short trailing reply back into the previous span improved segmentation cleanliness

After this fix, the best v2 run reached:

- overall F1: `51.06%`
- NemoriF1: `47.34%`
- LLM: `65.33%`

After later rolling back failed retrieval-surface and title experiments, the current best v2 run improved again to:

- overall F1: `52.03%`
- NemoriF1: `49.21%`
- LLM: `66.00%`
- single-hop: `74.49%`
- temporal: `24.90%`

### 3. Keeping non-LLM provisional content

The deterministic, transcript-like provisional content consistently behaved better than switching back to LLM-generated episodic content in the hot path.

This matches the v2 design goal in `docs/internal`:

- provisional projection must be immediately retrievable
- but it should stay light and stable
- text enrichment should not dominate the hot path

## What Did Not Help

### 1. LLM-based episodic `title/content` from the old `main` style

We tried restoring `main`-style LLM-generated episodic title/content while keeping v2 boundaries.

Compared to the best v2 run, this regressed:

- overall F1: `51.06% -> 49.41%`
- multi-hop: `26.38% -> 21.28%`
- open-domain: `20.82% -> 7.19%`

Temporal moved slightly up, but the overall tradeoff was clearly bad.

Observed failure mode:

- LLM observation text was better at temporal narration
- but it abstracted away too many lexical anchors
- repeated-theme retrieval got worse
- open-domain and multi-hop suffered heavily

Conclusion:

- do not use `main`-style LLM title/content as the default retrieval surface for v2

### 2. Replacing retrieval surface with compressed `retrieval_text`

We tried a dual-representation experiment:

- keep `content` for display
- add a short `retrieval_text`
- switch embedding and BM25 to `title + retrieval_text`

This regressed hard:

- overall F1 dropped to `44.25%`
- temporal: `19.02%`
- open-domain: `15.24%`
- multi-hop: `17.73%`

Failure mode:

- compressed retrieval text did not preserve enough lexical coverage
- repeated-theme collision did not disappear
- but many useful raw lexical anchors were lost

Conclusion:

- `retrieval_text` may be useful as an auxiliary signal later
- it should not replace the main retrieval surface

### 3. Evidence-sketch re-rank plus compressed deterministic content

We then tried a lighter experiment:

- keep non-LLM projection
- compress episodic content to a few selected messages
- add an evidence-sketch re-rank on top of episodic retrieval

This was also clearly bad:

- overall F1: `43.03%`
- temporal: `18.56%`
- open-domain: `10.00%`
- single-hop: `63.07%`

Failure mode:

- compressing episodic content reduced coverage too aggressively
- evidence-sketch re-rank did not compensate for the lost information
- all categories degraded, not just temporal

Conclusion:

- do not compress provisional `content` to a small selected-message sketch
- do not add re-rank heuristics on top of a weakened retrieval surface

### 4. Smarter deterministic title selection

We also tried a narrower follow-up:

- keep full deterministic non-LLM `content`
- remove re-rank changes
- but replace the simple provisional title with a scored clause-selection heuristic

This also regressed.

Representative numbers on `conv-47`:

- best stable v2 run: overall F1 `51.06%`
- after title tweak: overall F1 `47.13%`
- multi-hop: `26.38% -> 18.99%`
- temporal: `24.34% -> 18.32%`
- open-domain: `20.82% -> 5.38%`

Failure mode:

- even though `content` stayed full, the title still affected the retrieval surface enough to hurt ranking
- the “smarter” title was less stable than the plain first-message seed
- open-domain and temporal retrieval became much worse

Conclusion:

- do not try to make provisional title selection smarter in the hot path without very strong evidence
- the simple seed-title approach is safer for now

## Case Studies

### `conv-42`

`conv-42` exposed a repeated-theme retrieval problem:

- Joanna's writing / movies
- Nate's tournaments / gaming
- turtles
- desserts

The common failure was not "wrong topic", but "right topic, wrong instance".

Examples:

- correct theme, wrong movie
- correct tournament line, wrong tournament
- retrieved a nearby repeated episode instead of the exact one

This suggests:

- the main weakness is repeated-theme disambiguation
- not simply missing retrieval or missing segmentation

### `conv-44`

`conv-44` exposed a related issue:

- dog-related topics repeated many times
- girlfriend/weekend/outdoor activity topics repeated many times

The common failure mode was:

- wide semantic or episodic memory matched the right area
- but a more generic fact overrode the exact detail

Examples:

- `about an hour` vs generic "multiple times a day"
- `twice a week` getting lost behind broad dog-walking facts
- correct month/year or ordering detail being flattened away

This points to a detail-preservation problem more than a segmentation problem.

## Current Best Understanding

At this point the evidence is fairly consistent:

1. The v2 segmentation direction is correct.
   - Bigger episodic spans are better than the earlier fragmented v2 states.
   - `PredictCalibrate` gating is a clear win.

2. The provisional retrieval surface should stay mostly deterministic.
   - LLM-generated episodic content was not a stable win.
   - Compressed retrieval-only text was also not a win.

3. The main remaining problem is repeated-theme disambiguation, especially for temporal questions.
   - This is not fully solved by changing the segmentation shape.
   - It is also not solved by replacing the retrieval surface with shorter summaries.

4. `temporal` is still the main weak category.
   - Interestingly, temporal F1 is often much lower than LLM judge.
   - This suggests some errors are about choosing the wrong instance or mismatching exact gold phrasing, not complete semantic failure.

## Current Recommended Default

Keep the system close to the best v2 run:

- v2 span-first segmentation
- larger episodic spans
- `PredictCalibrate` gating
- deterministic provisional title/content
- full non-LLM episodic content, not compressed sketches
- no retrieval-text replacement
- no evidence-sketch re-rank in the default path

At the moment, the best validated default is simply the rolled-back stable version:

- plain seed-based provisional title
- full deterministic non-LLM provisional content
- no extra retrieval-surface experimentation in the hot path

## Recommended Next Steps

Do not continue with large retrieval-surface experiments first.

Instead:

1. Keep the current stable retrieval surface and rerun benchmarks after each small change.
2. Improve deterministic title selection only, without shrinking `content`.
3. Study repeated-theme failures in `conv-42` and `conv-44` at the retrieved-episode level.
4. If a re-rank experiment is retried later, apply it only as a weak tie-break over stable top-k candidates.
5. Treat temporal improvement as a separate problem from segmentation quality.

## Status After Reverts

After the last rollback, the code returned to the safer direction:

- full deterministic non-LLM provisional `content`
- no evidence-sketch re-rank
- no compressed retrieval-only surface
- simple seed-based provisional title

That rollback path is also what produced the current best v2 score:

- overall F1: `52.03%`
- NemoriF1: `49.21%`
- LLM: `66.00%`

This state has been validated with:

- `cargo test -p plastmem_core`
- `cargo test -p plastmem_worker event_segmentation`
- `cargo check`
