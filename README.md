# Null IDE

**A calm, fast code editor where nothing gets in your way.**

Null is a new IDE built around one idea: the screen belongs to your code. It aims for a
first-class, polished experience — instant, fluid, beautiful — and keeps AI within reach
without ever making it the center of the room.

> Status: **pre-alpha**. Nothing usable yet — the foundations are being laid.

## Principles

- **Code first.** No permanent side panels competing for space. Chrome fades while you type.
- **AI on demand, never in charge.** Summon it inline when you want it, dismiss it with
  `Esc`. It never edits your code without showing you a diff first.
- **Speed is a feature.** Near-instant startup, no dropped frames, minimal input latency.
- **Crafted details.** Motion, typography and themes (including a true-black OLED theme)
  are treated as core features, not polish for later.
- **Bring your own model.** Local models or your own API key — your choice.

## Planned stack

- [Rust](https://www.rust-lang.org/) + [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui) for GPU-accelerated UI
- [tree-sitter](https://tree-sitter.github.io/) for syntax and structure
- [LSP](https://microsoft.github.io/language-server-protocol/) / [DAP](https://microsoft.github.io/debug-adapter-protocol/) for language intelligence and debugging

## Platforms

| Platform | Status |
|---|---|
| macOS (Intel & Apple Silicon) | Primary development target |
| Windows 10 / 11 | Planned |
| Linux | Planned |

## Workflow

- `main` — stable; only receives merges from `dev`.
- `dev` — integration branch; all feature work merges here first.
- Feature branches — branch off `dev`, named `feature/<topic>`, `fix/<topic>`, etc.,
  and open a pull request back into `dev`.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
