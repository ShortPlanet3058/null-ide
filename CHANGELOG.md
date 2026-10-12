# Changelog

## Unreleased

- **Several folders in one window**: Add Folder to Window…, folders dropped on the files, or
  `null api web`. Each folder is a heading in the files; ⌘P, search, symbols and git's marks
  cover them all; each gets its own language servers; git follows the file you're in, with a
  picker of the folders' repositories in the Git panel. The window remembers its folders.

## 0.1.0

The first release of Null: a calm, fast code editor where the screen belongs to your code,
with AI hidden until you call it (and nowhere at all when it's off).

**What's in it**

- An editor made for long days: six themes (OLED black with an amber cursor by default),
  bars that fade while you type, multiple cursors, folding, sticky scroll, Vim mode, and the
  keys of VS Code, JetBrains, Sublime Text or Zed if your hands know them.
- Language servers found or installed for you (Rust, Python, TypeScript, Go, C/C++ and
  more): completion, go to definition, references, rename, quick fixes, inlay hints.
- Syntax highlighting for fourteen languages, and basic colouring for a dozen more.
- ⌘P / Ctrl+P to any file, ⌘K / Ctrl+K for quick settings and every command, project-wide
  search and replace, an outline, a terminal, tasks, tests found and run from the code, and
  a debugger (lldb-dap).
- Git in one panel: changes to commit (whole files or parts), history with its graph,
  branches, merges and rebases with conflicts resolved in place, and GitHub's pull requests
  and issues.
- Markdown written well: a live preview, tables, footnotes, formulas and Mermaid diagrams,
  smart quotes, spelling (on the Mac).
- Big files open at once; a 42,000-file project lists in a quarter of a second.
- AI, optional and off by default: ⌘I / Ctrl+I to change code shown as a diff to keep or
  undo, questions about a file, commit messages, and bigger tasks for Claude Code or Codex,
  reviewed change by change. With your Claude or ChatGPT plan, an API key (several free
  tiers), or Ollama on your own computer.

**Downloads**

- **macOS** (Intel and Apple silicon, macOS 11 or later): `Null-0.1.0.dmg`. Drag Null to
  Applications. This build isn't notarised by Apple yet: the first time, right-click Null
  and choose Open.
- **Windows** (10 and 11, x86-64): `null-0.1.0-windows-x86_64.zip`. Unzip it anywhere and
  run `null.exe`; in Null, "Install Shell Command" adds `null` for your terminals.
- **Linux** (x86-64, X11 or Wayland): `null-0.1.0-linux-x86_64.tar.gz`. Unpack it and run
  `./install.sh`, which puts Null in `~/.local/bin` and in your applications menu.

**Known limits**

- Windows and Linux are new: Null is built and tested on both by CI, but has been used
  every day only on the Mac so far. Please report what you find.
- Spell checking and the colour picker use the Mac's own; on Windows and Linux they aren't
  there yet.
- Debugging needs lldb-dap (with Xcode on the Mac, LLVM on Windows, LLDB on Linux).
