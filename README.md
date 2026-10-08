# Null IDE

**A calm, fast code editor where nothing gets in your way.**

Null is a new IDE built around one idea: the screen belongs to your code. It aims for a
first-class, polished experience — instant, fluid, beautiful — and keeps AI within reach
without ever making it the center of the room.

> Status: **pre-alpha**, used daily on macOS. An editor with language servers, git, a
> terminal, tests and a debugger, syntax highlighting for Rust, Python, JavaScript,
> TypeScript, JSON, TOML, Markdown, HTML, CSS, Go, C, C++, YAML and shell scripts (and basic
> colouring for Swift, Kotlin, Java, C#, Dart, Ruby, PHP, Lua, SQL, Dockerfiles, Makefiles, XML,
> and CSV/TSV by column), and AI that stays out of the way until called.

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
| ⌘F | find in the file (with lines selected, only in them); ↑ ↓ bring back earlier searches, ⌥↵ puts a cursor on every match; ⌘E searches for the selection, ⌘G the next |
| ⌘⇧F / ⌘⇧H | search / replace across the project (the chevron also picks which files: `*.rs, src/, !tests`; **Tabs**, only the files open) |
| replace in lower case | each match keeps its case: `user` → `client` makes `User` `Client` and `USER` `CLIENT` (typed with capitals, it goes in as typed) |
| ⌃- / ⌃⇧- | back / forward to where you were; ⇧⌘⌫ to the last edit |
| ⌃⌘→ / ⌃⌘← | move the tab to the right or left side (split view) |
| right-click a tab → Pin Tab | it stays first, out of "Close Others" and "Close All"; its pin unpins it |
| ⌘⇧N / ⌥⌘O | a new window / a recent project |
| drop from the Finder | files open; on a folder in the files, they're copied there; on the terminal, their paths are typed in |
| ⌥⌘↵ | focus mode: only the code |
| ⌥⌘I | an AI task, reviewed change by change (with Claude Code or Codex) |

Coming from another editor? Pick its shortcuts (VS Code, JetBrains, Sublime Text or Zed) on
the welcome screen or in Settings → Keyboard. Your own go in settings.json, over those:

```json
"keys": { "ctrl-cmd-l": "select all occurrences", "cmd-d": null }
```

A command is named as ⌘K lists it (or as in the code, `editor::SelectAllOccurrences`), and
`null` takes a shortcut away, everywhere in the window (the terminal and the files too).
Menus and ⌘K show your keys; one that can't be understood, or that would type a character
(`x`, a space), is named when settings.json is saved.

Shaders are compiled when the app starts (GPUI's `runtime_shaders` feature), so the build
doesn't need Xcode's separate Metal Toolchain download.

## Writing

- **⌘.** offers the language server's quick fixes at the caret (a missing import, match
  arms…), and with AI on, Fix with AI for an error. **Organize Imports** (⌘K) has the server
  sort the file's imports and drop the unused ones, where it can (TypeScript, Go, Python). **F8** goes from problem to problem.
- Blocks fold from the gutter's arrow or ⌥⌘← / ⌥⌘→ (their first and closing lines stay);
  **Fold Level 1/2/3** (⌘K) folds every block that deep, for the file's outline.
- Multiple cursors (⌘D, ⌘⇧-click), a box with ⌥⇧-drag, cursors at line ends (⌥⇧I);
  expand the selection by syntax, move and duplicate lines, join, sort, change case.
- ⌃⌥↑ / ⌃⌥↓ step the number at the caret (or the next one on the line) up or down, by ten
  with ⇧; on a decimal's fraction, by its last place (`0.5` → `0.6`). `007`, `0x0F` and
  `1.2.3` stay written that way. On `true`, `yes` or `on`, the value flips.
- In HTML and JSX, typing the `>` of a tag adds its closing tag, `</` finishes the one
  still open, the caret in a tag's name outlines its pair, renaming a tag renames the pair,
  and Enter between a tag and its closing one
  opens it onto lines of their own. ⇥ after an Emmet abbreviation writes its
  tags out: `ul>li.item*3`, `.card>h2{Title}+p`, `a`, `input[type=email]`; in HTML, and in
  JSX inside an element (with `className`, `htmlFor`, `<img />`). `<` typed over selected
  text wraps it in a tag, its name typed in both ends at once (whole lines get it on lines
  of its own).
- ⌘-click a web address or a file's path written in the text (`src/a.rs:12`, a README's
  links) to open it; anywhere else, ⌘-click goes to the definition.
- Files no language server knows (YAML, shell, a language not installed yet) still get
  suggestions: the file's own words, nearest first. In Markdown and text, only on ⌃Space.
  A path being written (`./`, `src/`, `img/`) lists what's in that folder.
- **Bookmarks**: ⌘F2 marks the caret's line (its number turns amber), F3 and ⇧F3 go round
  them, and **Bookmarks…** (⌘K) lists them in every open file. They move with the code and
  are kept with the project.
- **Your snippets**: a prefix typed offers its snippet among the suggestions, its places
  filled in with ⇥. **Edit Snippets…** (⌘K) opens the file for the current language; they're
  written as VS Code writes them, so its snippet files (and a project's
  `.vscode/*.code-snippets`) work as they are, variables too (`$TM_FILENAME`,
  `$CURRENT_YEAR`, `$CLIPBOARD`, `$UUID`…).
- **Paste from History…** (⌘K) brings back the last 20 things copied or cut. `TODO`, `FIXME`
  and `HACK` stand out in comments; **Find TODOs** lists them all.
- A `}`, `)` or `]` typed first on a line goes back to its opening line's indentation; in
  Python, the line after `return` (or `pass`, `break`…) steps out of the block, and `else:`,
  `elif`, `except`, `finally` go back to their `if` or `try` as their `:` is typed.
- Ruby, Lua and shell scripts get the same for their words: Enter after `do`, `then` or
  `def` goes in a level, and `end`, `fi`, `done` or `else` goes back to what it closes as
  it's typed (and back in if it was the start of `endpoint`). In YAML, Enter after `key:`
  goes in, and after `- name: web` lines up under `name`. When the word typed is already
  the suggestion, ↩ is just a new line.
- Pasted code takes the indentation of where it goes, its lines keeping theirs relative
  to each other; ⌥⇧⌘V pastes it as it was.
- Enter in a doc comment (`///`, `//!`, ` * ` in `/** */`) starts the next line with it; in a
  plain comment, only when it splits the comment in two.
- **Rewrap** (⌘K) refills a comment or a paragraph to the project's line length, keeping
  its `//`, `>` or list indent. The ⌃ keys of macOS text fields work too: ⌃A ⌃E, ⌃K and
  ⌃Y, ⌃T, ⌃O, ⌃L to center the caret's line, and ⌘J to bring the selection back into view;
  ⌃⌥← → go by the parts of a name (`parse` `Http` `Request`), ⌃⌥⇧ selecting, ⌃⌥⌫ deleting.
- Misspelled words in Markdown, text and comments get a faint wavy line, checked by the
  Mac's own speller in the languages set in System Settings (each line in the one it reads
  best in, so French and English mix); code is never marked. **⌘.** on one offers corrections.
- Colours written in CSS, HTML, scripts and theme files (`#f80`, `rgb()`, `hsl()`) show a
  small square of themselves just before; a click on it opens the Mac's colour panel, and the
  colour picked there is written back the way the first was (one undo takes it back).
- Code the language server says is unused (an import, a variable) is drawn faded, and a
  deprecated name struck through.
- CSV and TSV files: each column in a colour of its own, so one can be followed down the
  rows (quoted fields, `;` from a French spreadsheet, line breaks in quotes understood), and
  the status bar names the caret's column from the first line (`price (column 3)`).
- A file's language comes from its name, or else its first line: a script without an
  extension (`#!/usr/bin/env python3`, `#!/bin/bash`), an untitled file you paste into,
  `<?xml`, `<!DOCTYPE html>`. A click on the language's name in the status bar (or ⌘K
  "Language: …") puts the file in another; the choice is kept with the session. Saving an
  untitled file suggests a name from it: a note's first heading (`Trip to Lyon.md`), prose's
  first words, `untitled.py` for code.
- When the language server can say what each name is (rust-analyzer, clangd, Swift's
  sourcekit-lsp), its colours go over the grammar's: a type is coloured as a type, a call
  as a call, a macro as a macro, even where the grammar could only guess from how it's
  written (SwiftUI's `Text(…)` is a type, not a call).
- Spaces and tabs show as faint dots and dashes in a selection; Settings → Editor can show
  them at line ends too (where they're left by mistake), or always.
- Faint indent guides (the caret's block's a little brighter), sticky scroll (the enclosing lines stay at the top), the other uses
  of a name tinted, and a line at the length the project keeps to (from `.editorconfig`,
  rustfmt, Prettier, Black or Ruff). Each can be turned off. Wrapped lines can break at that
  line rather than at the window's edge (Settings → Editor → Wrap at the line guide).
- Files keep their own style: indentation, line endings, and encoding (UTF-8 with or
  without BOM, UTF-16, Windows-1252), shown in the status bar when not the usual.
- Unsaved work survives a crash, and quitting doesn't ask about it: it's kept, and comes
  back unsaved next time (Settings → Editor turns this off; closing a tab still asks). A
  file renamed or moved outside Null is followed; one deleted shows struck through, and
  saving puts it back. A file changed on disk under unsaved edits (a pull, an AI agent)
  asks before a save writes over it. Saves are whole or not at all, and keep the file's
  permissions and tags.
- **Revert to Saved** (File menu, ⌘K) takes the file back to its last save, as one step ⌘Z
  undoes.
- **Local history**: what each save replaced is kept for a month, git or not, and listed in
  **Show File History** with the commits, to compare with and take back.
- In the files, typing a name goes to it, as in the Finder; a file with an error in it shows
  red, its folders a red dot. **Find in Folder…** searches one folder from its menu.
  ⌘-click picks several files (⇧-click, all those between): their menu moves them to the
  Trash with one question, copies their paths, or compares two files; dragged, they all move
  (onto Markdown, a link each).
  Folders that each hold only the next open together, as one row (`src / main / java`):
  new files, drops and renames there go to the last of them.
- Selected text drags to where it's dropped (⌥ copies it); a file dragged from the files
  onto Markdown becomes a link, onto other code it opens.
- Images open as images; other files that aren't text are never saved over. Minified
  files with very long lines stay quick. Holding ⌥ over an image's path written in the text
  (`![](shot.png)`, `src="logo.svg"`) shows the image. HTML and SVG files open in the browser
  from ⌘K or their right-click menu.

## Markdown

**⌘⇧V** in a Jupyter notebook (`.ipynb`) shows it as it reads: text cells, code in its
language, and below each cell what it printed, set apart by a faint line, and its charts as pictures. In an SVG it shows the drawing its text makes, unsaved edits included (⎋ goes back to
the text). In Markdown, it shows the file as it reads (headings, lists, tables, code coloured, local
images; a task's box ticks with a click, and in the text with ⌘-click or ⇧⌘X, which also
makes lines tasks); **Open Markdown Preview to the Side** keeps it next to the source, following as you
scroll and type. Markdown and text wrap on their own setting (⌥Z switches it in one of
those files); Enter carries lists and quotes on, Tab nests an item, and numbered lists count
on by themselves when an item is added, moved or deleted. Images and files dropped
from the Finder become links where they land (copied next to the file when from outside
the project); an image pasted (a screenshot) is saved next to the file and linked; a web address
pasted over some words links them; text copied from a web page or a document (Notes, Pages,
Google Docs) pastes as Markdown, its headings, links, bold, lists and tables kept (⌥⇧⌘V for
the plain text); ⌥⇧F lines the tables up, and in a table ⇥ ⇧⇥ go from cell to cell
(⇥ in the last one adds a row). `*`, `_` or `~` typed over selected words wrap them (twice
for **bold**). **Insert Footnote** (⌘K) puts the next `[^n]` at the caret and starts its note
at the end, ⌃- coming back. The preview shows footnotes (`[^1]`, gathered at the end) and GitHub's
callouts (`> [!NOTE]`, `[!TIP]`, `[!WARNING]`…).

Links stay right: renaming or moving a file in Null rewrites the Markdown links to it; a link
to a file or `#section` that isn't there gets a wavy line, and **⌘.** offers the nearest
names; `](#` completes from the headings. Cells copied from a spreadsheet paste as a table,
and **Insert Table of Contents** (⌘K) lists the headings, kept up to date when run again.
**Export as HTML** (⌘K) writes the document as a page next to it and opens it in the browser;
**Copy as Rich Text** (⌘K) copies it (or the selection) to paste formatted in Mail, Notes or
Docs. In code, **Copy with Colours** does the same for slides and documents: the file or the
selection pastes into Keynote, Pages or Mail in the colours and font it shows in here.

For long writing, the status bar counts the words and the minutes they take to read. In
Settings, **typewriter scrolling** keeps the line being written in the middle of the window,
**dim other paragraphs** leaves only the one being written in full, **smart quotes and
dashes** curl quotes in the Mac's style and make `--` a dash (never in code), and line
numbers can show everywhere, only in code, or nowhere.

## Git

- **⌃⇧G** lists what changed since the last commit; each file opens with its changes to
  keep or take back one by one.
- **Commit** lists the files under the message: ⇥ leaves one out; with AI on, ⌘I writes the
  message from the changes. **Undo Last Commit** takes it back (not once pushed), its
  message waiting for the next. **Set Changes Aside** and **Bring Back Changes** stash and
  return them. **Push**, **Pull** and **Fetch**;
  ↑ and ↓ beside the branch count what's to push and pull (as last fetched: Null never
  goes to the network on its own). Switch or start a branch from the status bar.
- A click on a changed line's mark in the gutter shows the change, to keep (⇥) or take
  back (Esc). **Discard Changes…** in a file's right-click menu takes a whole file back
  (a new one goes to the Trash, never deleted).
- Who last changed the caret's line, faintly at its end. **Show File History**, and any
  commit's version compared with the file now; in every comparison, the words that changed
  in a line stand out. **Open Line on the Web** shows it on GitHub, GitLab or Bitbucket. Compare with Saved, the Clipboard, or another
  File, each difference kept or taken back.
- Merge conflicts: each side tinted, **⌘.** keeps one or both, **⌥F8** to the next.

## Running, testing, debugging

- **⌘⇧B** runs what the project defines (Cargo, package.json scripts, Make, just, Go).
- **⌥⌘T** runs the test at the caret (Rust, Go, pytest, vitest, jest), or all of a file's.
- The terminal (**⌃\`**, more with **⌃⇧\`**; a double-click on one's name renames it, and a
  tab's menu opens one in its file's folder): ⌘-click a `file:line:col` or a link in its
  output to open it, **⌘F** searches it. **Run Selection in Terminal** (⌘K) runs the lines
  selected, or steps through a script line by line.
  A command's errors and warnings (a compiler's `file:line:col: error`, rustc's, TypeScript's,
  pytest's, a Python traceback) join the Problems list (⌘⇧M) and F8 when it ends, in files that
  exist, until the next command. A command that ran 10 seconds or more says when it's done: while you're in another app the
  Dock's icon jumps once and the note waits for your return; with the terminal hidden, a
  line at the bottom says what finished and how long it took. While the terminal is hidden
  and something runs in it, the status bar says what (`cargo running`; a click shows it), and
  a terminal's tab gets a dot while it's busy behind another.
- **F5** debugs (lldb-dap, with Xcode): breakpoints with **F9**, conditions with a
  right-click, values shown faintly beside the code and in a panel. Under the variables,
  type an expression to watch (`letters * 2`): its value shows at every stop.

## Principles

- **Code first.** No permanent side panels competing for space. Chrome fades while you type.
- **AI on demand, never in charge.** Summon it inline when you want it, dismiss it with
  `Esc`. It never edits your code without showing you a diff first.
- **Speed is a feature.** Near-instant startup, no dropped frames, minimal input latency.
- **Crafted details.** Motion, typography and themes (including a true-black OLED theme,
  and light ones that can follow the Mac's light and dark) are treated as core features,
  not polish for later.
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
called from, ⌥F12 shows a definition where you are without going there; ⌥⇧F formats the file, and
"Format on save" does it on ⌘S (and "Format on paste", to pasted code, where the server formats
parts of files); ⌘⇧M, or the error count in the status bar, lists every
problem found.

Without a server, ⌥⇧F still formats JSON (Null lays it out itself: keys in their order,
numbers as written, the comments of `tsconfig.json` kept, the file's own indentation; broken
JSON is left alone with the line where it breaks) and lines up Markdown tables.

In Rust, **Expand Macro** (⌘K) shows what the macro at the caret turns into (`println!`, a
`derive`), from rust-analyzer, in the info card; **Go to Parent Module** goes to the `mod` line
that brings the file in, **Open Cargo.toml** to its crate's, and **Open Documentation** opens
the docs.rs (or standard library) page of the name at the caret.

In C, C++ and Objective-C, **⌃⌘↑** goes from a source file to its header and back (`app.cpp` ↔
`app.h`): clangd finds it anywhere in the project (`include/`), and without it, it's looked for
beside the file.

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
one in Settings → Appearance, or set `code_font` and `ui_font` in the JSON. Ligatures (`->`
as an arrow) show with fonts that keep them in their columns, such as JetBrains Mono, Fira
Code or Cascadia Code; Geist Mono's would shift the code, so Null leaves them off.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
