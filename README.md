# Null IDE

**A calm, fast code editor where nothing gets in your way.**

Null is a new IDE built around one idea: the screen belongs to your code. It aims for a
first-class, polished experience — instant, fluid, beautiful — and keeps AI within reach
without ever making it the center of the room.

> Status: **pre-alpha**, used daily on macOS. An editor with language servers, git, a
> terminal, tests and a debugger, syntax highlighting for Rust, Python, JavaScript,
> TypeScript, JSON, TOML, Markdown, HTML, CSS, Go, C, C++, YAML and shell scripts, and AI
> that stays out of the way until called.

## Building

Requires [Rust](https://rustup.rs) (stable) and, on macOS, Xcode.

```sh
cargo run                      # open the current folder
cargo run -- path/to/folder    # open another folder
cargo run -- path/to/file.rs   # open a file
```

### The Mac app

```sh
packaging/macos/bundle.sh             # builds target/release/Null.app
packaging/macos/bundle.sh --install   # and copies it to /Applications
```

The app has Null's icon, opens from the Dock and the Finder ("Open With → Null" for files
and folders), and takes files dropped on its Dock icon. **Null › Install Shell Command**
adds a `null` command: `null .` or `null src/main.rs` from any terminal opens them in the
running Null. The bundle is signed ad hoc, for this Mac; sharing it needs a Developer ID
signature. Everything the script uses comes with macOS and Rust.

Settings open with ⌘, / Ctrl+,. They're saved in `~/.config/null/settings.json`
(`%APPDATA%\Null\settings.json` on Windows), which can also be edited by hand
(“Edit as JSON…” in Settings).

Getting around:

| Shortcut | Does |
|---|---|
| ⌘P | go to a file, recent ones first |
| ⌘K | quick settings (theme, text size, line spacing, wrap, sidebar, terminal, AI…) changed right in the list, and every command once you type |
| ⌘, | all settings |
| ⌃G | go to a line |
| ⌘⇧O / ⌘T | go to a function or type, in the file / in the project |
| ⌘P `file:42:7` | open a file at a place, as compilers print it |
| ⌘F | find in the file; ↑ ↓ bring back earlier searches; ⌘E searches for the selection, ⌘G the next |
| ⌘⇧F / ⌘⇧H | search / replace across the project (the chevron also picks which files: `*.rs, src/, !tests`) |
| ⌃- / ⌃⇧- | back / forward to where you were |
| ⌃⌘→ / ⌃⌘← | move the tab to the right or left side (split view) |
| ⌘⇧N / ⌥⌘O | a new window / a recent project |
| drop from the Finder | files open; on a folder in the files, they're copied there; on the terminal, their paths are typed in |
| ⌥⌘↵ | focus mode: only the code |
| ⌥⌘I | an AI task, reviewed change by change (with Claude Code or Codex) |

Coming from another editor? Pick its shortcuts (VS Code, JetBrains, Sublime Text or Zed) on
the welcome screen or in Settings → Keyboard.

Shaders are compiled when the app starts (GPUI's `runtime_shaders` feature), so the build
doesn't need Xcode's separate Metal Toolchain download.

## Writing

- **⌘.** offers the language server's quick fixes at the caret (a missing import, match
  arms…), and with AI on, Fix with AI for an error. **F8** goes from problem to problem.
- Multiple cursors (⌘D, ⌘⇧-click), a box with ⌥⇧-drag, cursors at line ends (⌥⇧I);
  expand the selection by syntax, move and duplicate lines, join, sort, change case.
- In HTML and JSX, typing the `>` of a tag adds its closing tag, `</` finishes the one
  still open, and renaming a tag renames its pair.
- ⌘-click a web address or a file's path written in the text (`src/a.rs:12`, a README's
  links) to open it; anywhere else, ⌘-click goes to the definition.
- Files no language server knows (YAML, shell, a language not installed yet) still get
  suggestions: the file's own words, nearest first. In Markdown and text, only on ⌃Space.
- Pasted code takes the indentation of where it goes, its lines keeping theirs relative
  to each other; ⌥⇧⌘V pastes it as it was.
- **Rewrap** (⌘K) refills a comment or a paragraph to the project's line length, keeping
  its `//`, `>` or list indent. The ⌃ keys of macOS text fields work too: ⌃A ⌃E, ⌃K and
  ⌃Y, ⌃T, ⌃O…
- Faint indent guides, sticky scroll (the enclosing lines stay at the top), the other uses
  of a name tinted, and a line at the length the project keeps to (from `.editorconfig`,
  rustfmt, Prettier, Black or Ruff). Each can be turned off.
- Files keep their own style: indentation, line endings, and encoding (UTF-8 with or
  without BOM, UTF-16, Windows-1252), shown in the status bar when not the usual.
- Unsaved work survives a crash. A file renamed or moved outside Null is followed; one
  deleted shows struck through, and saving puts it back.
- Images open as images; other files that aren't text are never saved over. Minified
  files with very long lines stay quick.

## Markdown

**⌘⇧V** shows a Markdown file as it reads (headings, lists, tables, code coloured, local
images; a task's box ticks with a click); **Open Markdown Preview to the Side** keeps it next to the source, following as you
scroll and type. Markdown and text wrap on their own setting (⌥Z switches it in one of
those files); Enter carries lists and quotes on, Tab nests an item. Images and files dropped
from the Finder become links where they land (copied next to the file when from outside
the project); an image pasted (a screenshot) is saved next to the file and linked; a web address
pasted over some words links them; ⌥⇧F lines the tables up, and in a table ⇥ ⇧⇥ go from cell to cell
(⇥ in the last one adds a row).

## Git

- **⌃⇧G** lists what changed since the last commit; each file opens with its changes to
  keep or take back one by one.
- **Commit** lists the files under the message: ⇥ leaves one out; with AI on, ⌘I writes the
  message from the changes. **Push** and **Pull**;
  ↑ and ↓ beside the branch count what's to push and pull (as last fetched: Null never
  goes to the network on its own). Switch or start a branch from the status bar.
- A click on a changed line's mark in the gutter shows the change, to keep (⇥) or take
  back (Esc).
- Who last changed the caret's line, faintly at its end. **Show File History**, and any
  commit's version compared with the file now.
- Merge conflicts: each side tinted, **⌘.** keeps one or both, **⌥F8** to the next.

## Running, testing, debugging

- **⌘⇧B** runs what the project defines (Cargo, package.json scripts, Make, just, Go).
- **⌥⌘T** runs the test at the caret (Rust, Go, pytest, vitest, jest), or all of a file's.
- The terminal (**⌃\`**, more with **⌃⇧\`**): ⌘-click a `file:line:col` or a link in its
  output to open it, **⌘F** searches it.
- **F5** debugs (lldb-dap, with Xcode): breakpoints with **F9**, conditions with a
  right-click, values shown faintly beside the code and in a panel.

## Principles

- **Code first.** No permanent side panels competing for space. Chrome fades while you type.
- **AI on demand, never in charge.** Summon it inline when you want it, dismiss it with
  `Esc`. It never edits your code without showing you a diff first.
- **Speed is a feature.** Near-instant startup, no dropped frames, minimal input latency.
- **Crafted details.** Motion, typography and themes (including a true-black OLED theme)
  are treated as core features, not polish for later.
- **Bring your own model.** Local models or your own API key — your choice.

## Built with

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

## Sessions

Null reopens each project as you left it: the same tabs, each with its caret and scroll,
the folders open in the tree, the terminal, and the window's place. Opened from the Finder
or the Dock, it comes back to the last project. Sessions are small files in Null's own folder
(`~/Library/Application Support/Null/sessions` on macOS; `NULL_DATA_DIR` overrides it).

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
⇧F12 lists where it's used (⌘-clicking a definition does too), ⌃⌥H where a function is
called from; ⌥⇧F formats the file, and
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

Null ships with [Geist Mono](https://github.com/vercel/geist-font) for code (the default) and
[Instrument Sans](https://github.com/Instrument/instrument-sans) for the interface, plus four
more code fonts so every platform has the same choice:
[Commit Mono](https://github.com/eigilnikolajsen/commit-mono),
[JetBrains Mono](https://github.com/JetBrains/JetBrainsMono),
[IBM Plex Mono](https://github.com/IBM/plex) and
[Source Code Pro](https://github.com/adobe-fonts/source-code-pro). Instrument Sans comes with its
italics, for emphasis in the Markdown preview. All are under the SIL Open
Font License 1.1 (see `assets/fonts/*/OFL.txt`). Any installed font can be used instead: pick
one in Settings → Appearance, or set `code_font` and `ui_font` in the JSON.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
