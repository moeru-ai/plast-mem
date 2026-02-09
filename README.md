# Plast Mem

Yet Another Memory Layer, inspired by Cognitive Science, designed for Cyber Waifu

## Core Design

These are the design goals for Plast Mem, some of which may not yet have been implemented.

### Self-hosted first

Plast Mem is built around self-hosting and does not try to steer you towards a website with a 'Pricing' tab.

Written in Rust, it is packaged as a single binary (or Docker image)
and requires only a connection to an LLM service (such as [llama.cpp](https://github.com/ggml-org/llama.cpp)) and a [ParadeDB](https://github.com/paradedb/paradedb) database to work.

<!-- ### Event Segmentation Theory -->

### FSRS

By introducing [FSRS (Free Spaced Repetition Scheduler)](https://github.com/open-spaced-repetition/fsrs4anki/wiki/The-Algorithm), we can determine when a memory should be forgotten.

After the memory has been retrieved, a separate reviewer scores the dialogue and updates the memory state.

## FAQ

### What is the current status of this project?

We have not yet released version 0.0.1 because the core functionality is incomplete. However, you are welcome to join us in developing it!

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
