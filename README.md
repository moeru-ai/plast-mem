# Plast Mem

Yet Another Memory Layer, inspired by Cognitive Science, designed for Cyber Waifu

## Core Design

These are the design goals for Plast Mem, some of which may not yet been implemented. See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for implementation details.

### Self-hosted first

Plast Mem is built around self-hosting and does not try to steer you towards a website with a 'Pricing' tab.

Written in Rust, it is packaged as a single binary (or Docker image)
and requires only a connection to an LLM service (such as [llama.cpp](https://github.com/ggml-org/llama.cpp)) and a [ParadeDB](https://github.com/paradedb/paradedb) database to work.

### Event Segmentation Theory

Conversations flow continuously, but human memory segments them into discrete episodes.
Plast Mem uses [Event Segmentation Theory](https://en.wikipedia.org/wiki/Psychology_of_film#Segmentation) to detect natural boundaries—topic shifts, time gaps, or message accumulation—and creates episodic memories at these boundaries. The dual-channel boundary detection (topic shift + surprise) is inspired by [HiMem](https://arxiv.org/abs/2410.21385).

### FSRS

By introducing [FSRS (Free Spaced Repetition Scheduler)](https://github.com/open-spaced-repetition/fsrs4anki/wiki/The-Algorithm), we can determine when a memory should be forgotten.

Retrieval records candidate memories for review; when the conversation is later segmented,
An LLM evaluates each memory's relevance (Again/Hard/Good/Easy) and updates FSRS parameters accordingly.

## FAQ

### What is the current status of this project?

We have not yet released version 0.0.1 because the core functionality is incomplete. However, you are welcome to join us in developing it! See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

#### Roadmap

- v0.0.1 - Episodic Memory, FSRS
- v0.1.0 - Semantic Memory
<!-- - v1.0.0 - ...Graph? Fact Version Control? maybe we can do more... -->

### Is it related to Project AIRI's Alaya?

No, but I might draw inspiration from some of it - or I might not.

## License

[MIT](LICENSE.md)

### Acknowledgments

This project is inspired by the design of:

- [Nemori: Self-Organizing Agent Memory Inspired by Cognitive Science](https://arxiv.org/abs/2508.03341)
- [HiMem: A Hierarchical Memory Framework for LLM-Based Agents](https://arxiv.org/abs/2410.21385) — dual-channel segmentation design
