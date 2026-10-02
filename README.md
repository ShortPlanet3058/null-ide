# Null IDE

**A calm, fast code editor where nothing gets in your way.**

Null is a new IDE built around one idea: the screen belongs to your code. It aims for a
first-class, polished experience — instant, fluid, beautiful — and keeps AI within reach
without ever making it the center of the room.

> Status: **pre-alpha**. Opens a folder with a file tree, tabs, a terminal and project
> search, with syntax highlighting for Rust, Python, JavaScript, TypeScript, JSON, TOML,
> Markdown, HTML, CSS, Go, C, C++, YAML and shell scripts.

## Building

Requires [Rust](https://rustup.rs) (stable) and, on macOS, Xcode.

```sh
cargo run                      # open the current folder
cargo run -- path/to/folder    # open another folder
cargo run -- path/to/file.rs   # open a file
```

Settings open with ⌘, / Ctrl+,. They're saved in `~/.config/null/settings.json`
(`%APPDATA%\Null\settings.json` on Windows), which can also be edited by hand
(“Edit as JSON…” in Settings).

One palette does everything; the first character picks what it searches:

| Type | Does | Shortcut |
|---|---|---|
| a name | open a file (recent ones first) | ⌘P |
| `>` | run a command, grouped by category | ⌘⇧P |
| `:` | go to a line | ⌃G |
| `?` | ask the AI about the open file | |

Shaders are compiled when the app starts (GPUI's `runtime_shaders` feature), so the build
doesn't need Xcode's separate Metal Toolchain download.

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

## AI (optional, off by default)

AI only appears when you call it: Cmd/Ctrl+I to change the code at the caret (shown as a
diff you accept or reject), or `?` in the palette to ask about the open file.
Choose where answers come from in Settings → AI:

| Provider | Needs |
|---|---|
| NVIDIA | an NVIDIA API key |
| Ollama | Ollama running locally |
| OpenAI-compatible | any compatible endpoint (`base_url` + `model` in settings), key optional |
| Claude API | an Anthropic API key |
| Claude Code | the `claude` CLI, signed in (uses your Claude plan) |
| Codex | the `codex` CLI, signed in (uses your ChatGPT plan) |

API keys are stored in the system keychain (Settings → AI → API key), or read from
`NVIDIA_API_KEY` / `OPENAI_API_KEY` / `ANTHROPIC_API_KEY`; they never go in the settings
file. Claude Code and Codex run with every tool disabled, so they can only answer; Null
applies changes itself. Models and addresses can be changed in Settings → AI.

## Fonts

Null ships with [Geist Mono](https://github.com/vercel/geist-font) for code and
[Instrument Sans](https://github.com/Instrument/instrument-sans) for the interface, both under
the SIL Open Font License 1.1 (see `assets/fonts/*/OFL.txt`). Any installed font can be used
instead: pick one in Settings → Appearance, or set `code_font` and `ui_font` in the JSON.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
