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

Getting around:

| Shortcut | Does |
|---|---|
| ⌘P | go to a file, recent ones first |
| ⌘K | quick settings (theme, text size, wrap, sidebar, terminal, AI…) changed right in the list, and every command once you type |
| ⌘, | all settings |
| ⌃G | go to a line |

Coming from another editor? Pick its shortcuts (VS Code, JetBrains, Sublime Text or Zed) on
the welcome screen or in Settings → Keyboard.

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

## Code intelligence

Completions, errors, info on hover and go to definition come from language servers. Null
knows which one each language needs, finds it if it's installed (Xcode's clangd and
sourcekit-lsp included), and otherwise offers to install it with one click, from the status
bar or Settings → Languages, into its own folder rather than the system.

| Language | Server | Installed with |
|---|---|---|
| Rust | rust-analyzer | rustup |
| Python | pyright | npm (needs Node.js) |
| TypeScript, JavaScript | typescript-language-server | npm |
| C, C++, Objective-C | clangd | comes with Xcode's command line tools |
| Go | gopls | go |
| Swift | sourcekit-lsp | comes with Xcode |
| Shell | bash-language-server | npm |
| HTML, CSS, JSON | vscode-langservers-extracted | npm |
| YAML | yaml-language-server | npm |

With a server running: ⌘R (or F2) renames a symbol everywhere, right where it's written;
⇧F12 lists where it's used (⌘-clicking a definition does too); ⌥⇧F formats the file, and
"Format on save" does it on ⌘S; ⌘⇧M, or the error count in the status bar, lists every
problem found.

## AI (optional, off by default)

AI is a tool: it shows up only when you call it, right in the code, and a switch (⌘K or
Settings) turns it off entirely.

- **⌘I** opens a one-line field between your lines. Describe a small change (or press Enter
  on an error to fix it): it's written into the file as a diff, with removed lines struck
  through and new ones tinted. ⇥ keeps it, Esc undoes it, ⌘I adjusts it.
- **Questions** (typed in ⌘I, or “Ask About This File” in ⌘K) get a short note under the code
  they're about. Esc closes it.
- **⌘I writes in place**: the new code appears line by line as it's written, over the
  dimmed code it replaces.
- **Suggestions while typing** (Settings → AI, off by default): names from the file appear
  the moment you type two letters, then the AI's guess when you pause, streamed in. ⇥ takes
  it, ⌥→ takes the next word, ⌘→ the rest of the line, ⌥⇥ shows another option; typing along
  keeps it, and deleting back brings an earlier one straight back. It knows what the whole
  project defines (a local index, no AI) and the other open files. They come from a code model made to fill in the middle
  (StarCoder2 on NVIDIA, Qwen2.5-Coder on Ollama; changeable in Settings → AI), which sees the
  file and the files it includes or imports, and can write a whole body after `{` or `:`.
  `NULL_AI_LOG=<file>` logs how long each request takes.

Choose where answers come from in Settings → AI:

| Provider | Cost | Needs |
|---|---|---|
| Claude Code | your Claude plan (Pro or Max) | the `claude` command, signed in |
| Codex | your ChatGPT plan | the `codex` command, signed in |
| Mistral | free tier | a key from console.mistral.ai (Codestral gives the best suggestions) |
| Groq | free tier | a key from console.groq.com |
| Gemini | free on Flash models | a key from aistudio.google.com |
| OpenRouter | free `:free` models | a key from openrouter.ai |
| NVIDIA | free credits | a key from build.nvidia.com |
| Ollama | free, local | Ollama running on this computer |
| Claude API | paid per use | a key from console.anthropic.com |
| OpenAI | paid per use | a key from platform.openai.com (or any compatible server) |

Each provider comes with recommended models (one click in Settings → AI) and a thinking
level: quick by default, more for harder questions. Suggestions while typing always use the
least thinking, and their own model, a fill-in-the-middle code model where there is one.

API keys are read from the provider's usual variable (`MISTRAL_API_KEY`, `GROQ_API_KEY`,
`GEMINI_API_KEY`, `OPENROUTER_API_KEY`, `NVIDIA_API_KEY`, `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`) when set,
otherwise from the system keychain (Settings → AI → API key), at most once per launch; they
never go in the settings file. (A development build is a new binary each time, so macOS asks
before letting it read the keychain; set the environment variable to avoid that.) Claude Code and Codex run with every tool disabled, so they can only answer; Null
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
