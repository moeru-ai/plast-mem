# Research Log

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

1. The embedding dimenstion should be at least 1536 rather than 1024, using `text-embedding-3-small`.
2. The prompt for generating episodic memory's summary/content is bad.
3. Episodic memory vector search should use both title and content/summary rather than summary alone
4. The prompt for benchmark QA is bad
5. The semantic memory is to abstract and not helpful for the QA
6. Event segmentation has bad quality

Here are the comparison results for the first 4 hypothesis:

### H4 The prompt for benchmark QA is bad

After enhanced the QA prompt, the results:

```
── Results ──────────────────────────────────
By sample:

Sample conv-26
  Overall F1:   28.15%  (n=152)
  Overall Nemori F1: 25.37%
  Overall LLM:  0.00%

  Cat 1 (multi-hop   ):  F1=13.97%  NemoriF1=19.02%  LLM=0.00%  (n=32)
  Cat 2 (single-hop  ):  F1=21.24%  NemoriF1=11.85%  LLM=0.00%  (n=37)
  Cat 3 (temporal    ):  F1=22.31%  NemoriF1=11.74%  LLM=0.00%  (n=13)
  Cat 4 (open-domain ):  F1=39.36%  NemoriF1=37.95%  LLM=0.00%  (n=70)

Sample conv-30
  Overall F1:   27.26%  (n=81)
  Overall Nemori F1: 23.86%
  Overall LLM:  0.00%

  Cat 1 (multi-hop   ):  F1=22.18%  NemoriF1=31.46%  LLM=0.00%  (n=11)
  Cat 2 (single-hop  ):  F1=28.97%  NemoriF1=16.79%  LLM=0.00%  (n=26)
  Cat 4 (open-domain ):  F1=27.52%  NemoriF1=26.13%  LLM=0.00%  (n=44)

Sample conv-41
  Overall F1:   28.63%  (n=152)
  Overall Nemori F1: 29.75%
  Overall LLM:  0.00%

  Cat 1 (multi-hop   ):  F1=29.01%  NemoriF1=37.10%  LLM=0.00%  (n=31)
  Cat 2 (single-hop  ):  F1=18.44%  NemoriF1=16.51%  LLM=0.00%  (n=27)
  Cat 3 (temporal    ):  F1=6.29%  NemoriF1=5.06%  LLM=0.00%  (n=8)
  Cat 4 (open-domain ):  F1=33.77%  NemoriF1=33.56%  LLM=0.00%  (n=86)

Sample conv-42
  Overall F1:   29.13%  (n=199)
  Overall Nemori F1: 27.04%
  Overall LLM:  0.00%

  Cat 1 (multi-hop   ):  F1=14.00%  NemoriF1=16.85%  LLM=0.00%  (n=37)
  Cat 2 (single-hop  ):  F1=17.98%  NemoriF1=9.62%  LLM=0.00%  (n=40)
  Cat 3 (temporal    ):  F1=1.30%  NemoriF1=1.73%  LLM=0.00%  (n=11)
  Cat 4 (open-domain ):  F1=40.94%  NemoriF1=39.23%  LLM=0.00%  (n=111)

Sample conv-43
  Overall F1:   24.95%  (n=178)
  Overall Nemori F1: 25.18%
  Overall LLM:  0.00%

  Cat 1 (multi-hop   ):  F1=14.13%  NemoriF1=23.08%  LLM=0.00%  (n=31)
  Cat 2 (single-hop  ):  F1=15.74%  NemoriF1=13.48%  LLM=0.00%  (n=26)
  Cat 3 (temporal    ):  F1=5.08%  NemoriF1=4.01%  LLM=0.00%  (n=14)
  Cat 4 (open-domain ):  F1=32.91%  NemoriF1=31.40%  LLM=0.00%  (n=107)

Sample conv-44
  Overall F1:   20.85%  (n=123)
  Overall Nemori F1: 22.24%
  Overall LLM:  0.00%

  Cat 1 (multi-hop   ):  F1=17.46%  NemoriF1=22.24%  LLM=0.00%  (n=30)
  Cat 2 (single-hop  ):  F1=8.17%  NemoriF1=7.79%  LLM=0.00%  (n=24)
  Cat 3 (temporal    ):  F1=11.70%  NemoriF1=12.40%  LLM=0.00%  (n=7)
  Cat 4 (open-domain ):  F1=28.43%  NemoriF1=28.94%  LLM=0.00%  (n=62)

Sample conv-47
  Overall F1:   25.07%  (n=150)
  Overall Nemori F1: 25.39%
  Overall LLM:  0.00%

  Cat 1 (multi-hop   ):  F1=16.53%  NemoriF1=24.03%  LLM=0.00%  (n=20)
  Cat 2 (single-hop  ):  F1=9.80%  NemoriF1=5.85%  LLM=0.00%  (n=34)
  Cat 3 (temporal    ):  F1=12.45%  NemoriF1=12.49%  LLM=0.00%  (n=13)
  Cat 4 (open-domain ):  F1=35.35%  NemoriF1=35.74%  LLM=0.00%  (n=83)

Sample conv-48
  Overall F1:   22.60%  (n=191)
  Overall Nemori F1: 22.93%
  Overall LLM:  0.00%

  Cat 1 (multi-hop   ):  F1=8.78%  NemoriF1=13.32%  LLM=0.00%  (n=21)
  Cat 2 (single-hop  ):  F1=20.26%  NemoriF1=19.61%  LLM=0.00%  (n=42)
  Cat 3 (temporal    ):  F1=19.83%  NemoriF1=19.56%  LLM=0.00%  (n=10)
  Cat 4 (open-domain ):  F1=26.13%  NemoriF1=26.11%  LLM=0.00%  (n=118)

Sample conv-49
  Overall F1:   26.95%  (n=156)
  Overall Nemori F1: 28.22%
  Overall LLM:  0.00%

  Cat 1 (multi-hop   ):  F1=24.58%  NemoriF1=30.15%  LLM=0.00%  (n=37)
  Cat 2 (single-hop  ):  F1=21.23%  NemoriF1=18.64%  LLM=0.00%  (n=33)
  Cat 3 (temporal    ):  F1=23.46%  NemoriF1=24.05%  LLM=0.00%  (n=13)
  Cat 4 (open-domain ):  F1=31.35%  NemoriF1=32.31%  LLM=0.00%  (n=73)

Sample conv-50
  Overall F1:   14.16%  (n=158)
  Overall Nemori F1: 15.27%
  Overall LLM:  0.00%

  Cat 1 (multi-hop   ):  F1=11.70%  NemoriF1=13.69%  LLM=0.00%  (n=32)
  Cat 2 (single-hop  ):  F1=13.27%  NemoriF1=13.44%  LLM=0.00%  (n=32)
  Cat 3 (temporal    ):  F1=28.89%  NemoriF1=30.22%  LLM=0.00%  (n=7)
  Cat 4 (open-domain ):  F1=14.21%  NemoriF1=15.32%  LLM=0.00%  (n=87)

Overall
  Overall F1:   24.78%  (n=1540)
  Overall Nemori F1: 24.62%
  Overall LLM:  0.00%

  Cat 1 (multi-hop   ):  F1=17.27%  NemoriF1=22.78%  LLM=0.00%  (n=282)
  Cat 2 (single-hop  ):  F1=17.67%  NemoriF1=13.43%  LLM=0.00%  (n=321)
  Cat 3 (temporal    ):  F1=14.32%  NemoriF1=12.89%  LLM=0.00%  (n=96)
  Cat 4 (open-domain ):  F1=31.20%  NemoriF1=30.84%  LLM=0.00%  (n=841)
──────────────────────────────────────────────
```

The result is interesting:

*

Since the context retrieval has no change, the prompt only enhanced how the LLM uses the provided context / retrieved memory to answer the question.
