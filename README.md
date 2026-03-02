# Plast Mem

[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/moeru-ai/plast-mem)
[![License](https://badgen.net/github/license/moeru-ai/plast-mem)](LICENSE.md)

Yet Another Memory Layer, inspired by Cognitive Science, designed for Cyber Waifu

## Core Design

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for detailed implementation.

### Self-hosted first

Plast Mem is built around self-hosting and does not try to steer you towards a website with a 'Pricing' tab.

Written in Rust, it is packaged as a single binary (or Docker image)
and requires only a connection to an LLM service (such as [llama.cpp](https://github.com/ggml-org/llama.cpp), [Ollama](https://github.com/ollama/ollama)) and a [ParadeDB](https://github.com/paradedb/paradedb) database to work.

### Event Segmentation Theory

Conversations flow continuously, but human memory segments them into discrete episodes.
Plast Mem uses [Event Segmentation Theory](https://en.wikipedia.org/wiki/Psychology_of_film#Segmentation) to detect natural boundaries—topic shifts, time gaps, or message accumulation—and creates episodic memories at these boundaries.

Messages are accumulated in a queue and processed in batches. A single LLM call segments conversations into coherent episodes, each with a title, summary, and surprise level.

### Dual-Layer Memory Architecture

Plast Mem implements two complementary memory layers inspired by cognitive science:

**Episodic Memory** captures "what happened"—discrete conversation events with temporal boundaries. Each episode stores the original messages, an LLM-generated summary, and FSRS parameters for decay modeling.

**Semantic Memory** captures "what is known"—durable facts and behavioral guidelines extracted from episodes. Facts are categorized into 8 types (identity, preference, interest, personality, relationship, experience, goal, guideline) and use temporal validity instead of decay.

The **Consolidation Pipeline** (inspired by CLS theory) runs offline to extract semantic facts from unconsolidated episodes. When 3+ episodes accumulate or a flashbulb memory (surprise ≥ 0.85) occurs, an LLM processes the episodes against existing knowledge and performs new/reinforce/update/invalidate actions.

### Hybrid Retrieval

Memory retrieval combines multiple signals for relevance:

- **BM25** full-text search on summaries and keywords
- **Vector similarity** via embeddings (cosine distance)
- **Reciprocal Rank Fusion (RRF)** to merge keyword and semantic scores
- **FSRS retrievability** re-ranking for episodic memories (decay modeling)

The search returns the most relevant memories from both episodic and semantic layers, formatted as markdown for LLM consumption.

### FSRS

[FSRS (Free Spaced Repetition Scheduler)](https://github.com/open-spaced-repetition/fsrs4anki/wiki/The-Algorithm) determines when an episodic memory should be forgotten.

**Surprise-based initialization**: Episodes with high surprise (significant information gain) receive a stability boost, making them decay slower.

**Review mechanism**: Retrieval records candidate memories for review. When the conversation is later segmented, an LLM evaluates each memory's relevance (Again/Hard/Good/Easy) and updates FSRS parameters (stability, difficulty) accordingly. Semantic memories do not use FSRS—they remain valid until explicitly contradicted and invalidated.

## FAQ

### What is the current status of this project?

We have not yet released version 0.1.0 because the core functionality is incomplete. However, you are welcome to join us in developing it! See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

### Why not use a graph database?

While introducing graph structures can be beneficial, it also dramatically increases complexity. Plast Mem is written in Rust, which makes it difficult to easily reuse solutions like Graphiti.

Overall, I think the convenience of self-hosting is more important.

### Is it related to Project AIRI's Alaya?

No, but I might draw inspiration from some of it - or I might not.

### Which model should I use?

For locally running embedding models, we recommend [Qwen3-Embedding-0.6B](https://huggingface.co/Qwen/Qwen3-Embedding-0.6B) - its dimensionality meets requirements and delivers high-quality embeddings.

For other embedding models, simply ensure they can output vectors of 1024 dimensions or higher and support [MRL](https://huggingface.co/blog/matryoshka), like OpenAI's `text-embedding-3-small`.

For chat models, no recommendations are currently available, as further testing is still required.

## License

[MIT](LICENSE.md)

### Acknowledgments

This project is inspired by the design of:

- [Nemori: Self-Organizing Agent Memory Inspired by Cognitive Science](https://arxiv.org/abs/2508.03341)
- [HiMem: Hierarchical Long-Term Memory for LLM Long-Horizon Agents](https://arxiv.org/abs/2601.06377)
