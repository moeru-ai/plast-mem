# Plast Mem

[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/moeru-ai/plast-mem)
[![License](https://badgen.net/github/license/moeru-ai/plast-mem)](LICENSE.md)

Yet Another Memory Layer, inspired by Cognitive Science, designed for Cyber Waifu

## Core Design

These are the design goals for Plast Mem, some of which may not yet been implemented. See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for implementation details.

### Self-hosted first

Plast Mem is built around self-hosting and does not try to steer you towards a website with a 'Pricing' tab.

Written in Rust, it is packaged as a single binary (or Docker image)
and requires only a connection to an LLM service (such as [llama.cpp](https://github.com/ggml-org/llama.cpp)) and a [ParadeDB](https://github.com/paradedb/paradedb) database to work.

### Event Segmentation Theory

Conversations flow continuously, but human memory segments them into discrete episodes.
Plast Mem uses [Event Segmentation Theory](https://en.wikipedia.org/wiki/Psychology_of_film#Segmentation) to detect natural boundaries—topic shifts, time gaps, or message accumulation—and creates episodic memories at these boundaries.

### FSRS

By introducing [FSRS (Free Spaced Repetition Scheduler)](https://github.com/open-spaced-repetition/fsrs4anki/wiki/The-Algorithm), we can determine when a memory should be forgotten.

Retrieval records candidate memories for review; when the conversation is later segmented,
An LLM evaluates each memory's relevance (Again/Hard/Good/Easy) and updates FSRS parameters accordingly.

## FAQ

### What is the current status of this project?

We have not yet released version 0.1.0 because the core functionality is incomplete. However, you are welcome to join us in developing it! See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

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
