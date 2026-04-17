# Research Log

Status note: this is a historical benchmark log. It is not an authoritative
description of the current implementation. For current behavior, use the code in
`benchmarks/locomo/src` and the active docs in `docs/` and crate READMEs.

_We've already implemented the pipeline based on the correct idea, but why does the eval score not as high as SOTA such as Nemori?_

## 14 March 2026

Base result:

```
── Results ──────────────────────────────────
Overall F1:   25.52%  (n=1540)
Overall Nemori F1: 26.18%
Overall LLM:  0.00%

  Cat 1 (multi-hop   ):  F1=16.77%  NemoriF1=22.35%  LLM=0.00%  (n=282)
  Cat 2 (single-hop  ):  F1=16.48%  NemoriF1=13.28%  LLM=0.00%  (n=321)
  Cat 3 (temporal    ):  F1=12.20%  NemoriF1=11.95%  LLM=0.00%  (n=96)
  Cat 4 (open-domain ):  F1=33.43%  NemoriF1=34.02%  LLM=0.00%  (n=841)
──────────────────────────────────────────────
```
After comparing the current implementation of `plast-mem` and `benchmarks\locomo` to Nemori's repo, there are 6 hypothesis why our score is not high:

1. The embedding dimension should be at least 1536 rather than 1024, using `text-embedding-3-small`.
2. The prompt for generating episodic memory's summary/content is bad.
3. Episodic memory vector search should use both title and content/summary rather than summary alone
4. The prompt for benchmark QA is bad
5. The semantic memory is to abstract and not helpful for the QA
6. Event segmentation has bad quality

Here are the comparison results for the first 4 hypothesis:

### H4 The prompt for benchmark QA is bad

After enhanced the QA prompt, the results:

```
──────────────────────────────────────────────
Overall
  Overall F1:   25.32%  (n=1540)
  Overall Nemori F1: 26.67%
  Overall LLM:  0.00%

  Cat 1 (multi-hop   ):  F1=16.74%  NemoriF1=22.22%  LLM=0.00%  (n=282)
  Cat 2 (single-hop  ):  F1=17.41%  NemoriF1=17.44%  LLM=0.00%  (n=321)
  Cat 3 (temporal    ):  F1=15.14%  NemoriF1=14.70%  LLM=0.00%  (n=96)
  Cat 4 (open-domain ):  F1=32.37%  NemoriF1=33.04%  LLM=0.00%  (n=841)
──────────────────────────────────────────────
```

Not a very useful improvement. It proves that the QA prompt will not impact the overall performance too much. Since the context retrieval has no change, the prompt only enhanced how the LLM uses the provided context / retrieved memory to answer the question. Cat 3 got an increase compared to the base.

### H1 The embedding dimension should be at least 1536 rather than 1024, using `text-embedding-3-small`.

based on the modification on H4, I run the test on the first 9 samples (the last one sample `conv 50` was failed due to API request error). Then I separately ran the last sample, and here is the final results:

**Overall**

| 指标 | 分数 |
|---|---:|
| Overall F1 | 28.79% |
| Overall Nemori-style F1 | 30.16% |
| 总题数 | 1540 |

**By category**

| category | F1 | Nemori-style F1 | n |
|---|---:|---:|---:|
| Cat 1 multi-hop | 18.47% | 24.09% | 282 |
| Cat 2 single-hop | 20.27% | 20.09% | 321 |
| Cat 3 temporal | 18.95% | 18.04% | 96 |
| Cat 4 open-domain | 36.62% | 37.43% | 841 |

**by sample**

| sample | F1 | Nemori-style F1 |
|---|---:|---:|
| conv-26 | 29.98% | 30.83% |
| conv-30 | 31.87% | 31.74% |
| conv-41 | 29.07% | 32.03% |
| conv-42 | 26.29% | 27.78% |
| conv-43 | 27.40% | 29.55% |
| conv-44 | 35.50% | 37.12% |
| conv-47 | 29.41% | 30.53% |
| conv-48 | 24.71% | 25.33% |
| conv-49 | 30.31% | 31.60% |
| conv-50 | 28.10% | 29.25% |

In general, there is around 3% increase in the f1 score.

Since `Qwen` open source model doesn't support 1536 embedding dimension, the following experiments for other hypothesis will continue use 1024 dim.

### H2 & H3 Episodic memory related prompt is too bad; Episodic search needs both embedded `title + content`

| run | Overall F1 | Nemori-style F1 | delta |
|---|---:|---:|---:|
| original baseline | 25.52% | 26.18% | - |
| old 1024 run | 25.32% | 26.67% | -0.21pp vs baseline |
| 1536 run | 28.79% | 30.16% | +3.47pp vs 旧 1024 |
| New episodic implementation | 35.14% | 36.32% | +9.82pp vs 旧 1024 |

| category | current | prev 1024 | delta |
|---|---:|---:|---:|
| Cat 1 | 22.91% | 16.74% | +6.17pp |
| Cat 2 | 37.65% | 17.41% | +20.25pp |
| Cat 3 | 23.39% | 15.14% | +8.25pp |
| Cat 4 | 39.62% | 32.37% | +7.25pp |

| sample | current | prev 1024 | delta |
|---|---:|---:|---:|
| conv-26 | 37.92% | 30.02% | +7.90pp |
| conv-30 | 39.74% | 28.14% | +11.59pp |
| conv-41 | 31.80% | 28.92% | +2.89pp |
| conv-42 | 29.05% | 28.70% | +0.35pp |
| conv-43 | 41.09% | 27.31% | +13.78pp |
| conv-44 | 36.10% | 22.48% | +13.63pp |
| conv-47 | 36.67% | 23.88% | +12.79pp |
| conv-48 | 35.06% | 23.17% | +11.89pp |
| conv-49 | 35.28% | 26.70% | +8.58pp |
| conv-50 | 32.06% | 14.19% | +17.87pp |

| metric | current 1024+new episodic | previous 1536 run | delta |
|---|---:|---:|---:|
| Overall | 35.14% | 28.79% | +6.35pp |
| Cat 1 | 22.91% | 18.47% | +4.44pp |
| Cat 2 | 37.65% | 20.27% | +17.39pp |
| Cat 3 | 23.39% | 18.95% | +4.44pp |
| Cat 4 | 39.62% | 36.62% | +3.00pp |

This is a huge progress in the f1 score. Devil is in the details.

### H3.2 Time stamp + time accuracy requirement + time analysis framework

The enhancement in episodic memory generation has a huge help to the f1 score, so we keep it.

Time is always an important variable and the QA around it is always complicated and involves multiple transform or calculation.

```
Overall
  Overall F1:   32.13%  (n=1540)
  Overall Nemori F1: 33.31%
  Overall LLM:  0.00%

  Cat 1 (multi-hop   ):  F1=20.64%  NemoriF1=27.22%  LLM=0.00%  (n=282)
  Cat 2 (single-hop  ):  F1=32.60%  NemoriF1=32.34%  LLM=0.00%  (n=321)
  Cat 3 (temporal    ):  F1=17.07%  NemoriF1=15.55%  LLM=0.00%  (n=96)
  Cat 4 (open-domain ):  F1=37.52%  NemoriF1=37.75%  LLM=0.00%  (n=841)
──────────────────────────────────────────────
```

It's a bad change, revert. Nemori's episodic memory level single time stamp design is not suitable at all.

Then, we try to learn from Mastra: https://mastra.ai/research/observational-memory

Observation log + time model:

| run | Overall F1 | Nemori-style F1 | delta |
|---|---:|---:|---:|
| original baseline | 25.52% | 26.18% | - |
| 1536 run | 28.79% | 30.16% | +3.27pp vs baseline |
| new episodic implementation | 35.14% | 36.32% | +9.62pp vs baseline |
| Mastra-style OM content v1 | 37.82% | 38.81% | +12.30pp vs baseline |
| current semi-structured observation text | 41.58% | 42.42% | +16.06pp vs baseline |

| category | previous | current | delta |
|---|---:|---:|---:|
| Cat 1 | 22.77% | 23.08% | +0.31pp |
| Cat 2 | 41.15% | 44.08% | +2.93pp |
| Cat 3 | 20.19% | 24.09% | +3.90pp |
| Cat 4 | 43.61% | 48.83% | +5.22pp |

| sample | previous | current | delta |
|---|---:|---:|---:|
| conv-26 | 37.31% | 36.05% | -1.25pp |
| conv-30 | 39.96% | 45.18% | +5.22pp |
| conv-41 | 41.60% | 40.75% | -0.84pp |
| conv-42 | 32.46% | 39.92% | +7.46pp |
| conv-43 | 38.09% | 42.12% | +4.02pp |
| conv-44 | 42.19% | 45.64% | +3.45pp |
| conv-47 | 42.93% | 45.74% | +2.81pp |
| conv-48 | 32.45% | 44.67% | +12.22pp |
| conv-49 | 37.43% | 40.12% | +2.70pp |
| conv-50 | 38.68% | 37.97% | -0.71pp |

| sample | cat1 delta | cat2 delta | cat3 delta | cat4 delta | current cat3 | current cat4 |
|---|---:|---:|---:|---:|---:|---:|
| conv-26 | -1.26pp | -0.56pp | -5.88pp | -0.76pp | 18.06% | 42.27% |
| conv-30 | -4.75pp | +10.84pp | +0.00pp | +4.40pp | 0.00% | 38.06% |
| conv-41 | -5.37pp | +4.08pp | -3.68pp | -0.50pp | 12.36% | 47.26% |
| conv-42 | +5.12pp | +2.14pp | +23.38pp | +8.59pp | 25.40% | 50.65% |
| conv-43 | +4.24pp | -4.61pp | +0.14pp | +6.57pp | 13.50% | 51.42% |
| conv-44 | +0.09pp | -0.17pp | -1.35pp | +7.01pp | 8.39% | 62.78% |
| conv-47 | +4.58pp | +0.13pp | -1.54pp | +4.16pp | 30.93% | 56.76% |
| conv-48 | +1.14pp | +14.34pp | +10.55pp | +13.58pp | 29.71% | 48.08% |
| conv-49 | +1.03pp | +2.69pp | +11.37pp | +2.00pp | 47.19% | 43.95% |
| conv-50 | -4.10pp | -2.67pp | -0.40pp | +1.23pp | 19.84% | 43.23% |

This, is a huge progress. This is the best results we obtained ever. This is the first time our overall f1 score reached 40%!

### H3.1 Do we need BM25 for title + context only?

Based on this change, we further question, do we only need to use BM25 searching title and context, rather than title + context + raw message in order to avoid getting too much irrelevant details?

Yeah we still need...

### H3.3 Alias for `user` and `assistant`?

Yes we need, can't just use user/assistant

### H3.4 Segmentation prompt too bad?

Yeah, huge chunk segmentation is useless. Too much retrieved context leads to bad results.

### Final reach

We changed the episodic content format, only kept the key time information. We use the speakers' names rather than user/assistant. We shrunken our segmentation trigger window from 1 huge window into 30 messages, streaming ingest. We changed retrieval context presentation order: Episodic first and then semantic. Enhanced semantic memory generation prompt. Enhanced QA prompt for answering multi-hop questions.

NOW, we have:

```
Overall
  Overall F1:   54.51%  (n=1540)
  Overall Nemori F1: 51.84%
  Overall LLM:  0.00%

  Cat 1 (multi-hop   ):  F1=34.19%  NemoriF1=35.95%  LLM=0.00%  (n=282)
  Cat 2 (temporal    ):  F1=58.99%  NemoriF1=57.70%  LLM=0.00%  (n=321)
  Cat 3 (open-domain ):  F1=30.22%  NemoriF1=26.75%  LLM=0.00%  (n=96)
  Cat 4 (single-hop  ):  F1=62.38%  NemoriF1=57.80%  LLM=0.00%  (n=841)
──────────────────────────────────────────────
```

After four days of focused iteration, we achieved a SOTA-comparable result on our current LoCoMo setup, reaching the Nemori baseline level.
