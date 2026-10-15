# Changelog

## Unreleased

- AI reads less and pays less: ⌘I sends a long file's lines around the code with an outline
  of the rest instead of the whole file, the definitions the code uses from elsewhere in the
  project, and the project's AGENTS.md or CLAUDE.md; the field says what goes and about how
  many tokens. What stays the same is sent first and cached (Anthropic's cache is asked
  for; AI tasks cache their conversation as it grows).
- Tokens are counted: Settings → AI → AI usage, "AI Usage" in ⌘K, and an optional monthly
  limit.
- Language servers install on a computer with nothing else installed: the ones that need
  Node.js bring their own (downloaded into Null's folder, used only for them), and
  rust-analyzer, clangd (on Windows and Linux), Markdown's and Lua's are downloaded as
  programs. Every download is checked against the checksum Null was built with.
- New languages: Markdown, Lua, PHP, Svelte, Dockerfile, C# and Dart.
- Java, C#, Ruby, PHP and Lua are coloured by their real grammars (they had only basic
  colours); SCSS and LESS take CSS's.
- Language servers start with Null's own search path (Homebrew's Node, nvm's…), even when
  Null is opened from the Dock; uppercase extensions (`MAIN.CPP`) find their server.
- Suggestions while typing work with Claude Code and Codex: they come from a fast model of
  their own (Settings → AI → Suggestions come from), since the command line tools take
  seconds to answer. Null asks once where they should come from.
- Null is now proprietary software (All Rights Reserved): free to use as distributed by
  the author, its code public for transparency, but not to be redistributed, resold or
  reused (see LICENSE). 0.1.1 and earlier keep the license they were published under.
- A release lists each package once, under a name that stays the same from version to
  version.

## 0.1.1

What testing on fresh Windows, Ubuntu and Mac machines found, installers and uninstalling
everywhere, several folders in one window, and a new license.

**Windows, from its first tests**

- The window's minimise, maximise and close buttons work, and so do dragging, double-clicking
  and snap layouts.
- Ctrl+O opens a folder; File › Close Folder (also in Ctrl+K) leaves the window empty.
- Started from a shortcut, Null opens the last project; a file opened from the terminal goes
  to the window that holds it.
- Git, Node, Python and other tools are found in their usual Windows folders, and a missing
  Git is said as such, not as "not a repository".
- The welcome asks where AI answers come from on a step of its own, once AI is wanted.
- Shortcuts work on every keyboard: on digits and symbols they follow the key's place on
  the keyboard (on a French AZERTY: Ctrl+ù terminal, Ctrl+: comment, Ctrl+& first tab), and
  Ctrl+J shows and hides the terminal (Join Lines moves to Ctrl+Shift+J). On Linux an
  AZERTY's digit row works without Shift.

**Everywhere**

- **Several folders in one window**: Add Folder to Window…, folders dropped on the files, or
  `null api web`. Each folder is a heading in the files; ⌘P, search, symbols and git's marks
  cover them all; each gets its own language servers; git follows the file you're in, with a
  picker of the folders' repositories in the Git panel. The window remembers its folders.
- Null is now under the GNU General Public License 3.0 (0.1.0 stays MIT or Apache 2.0).
- Quick settings: − + and ← → are buttons; the ≡ button lists every command.
- Committing a file ends the changes shown in it.

**From testing on fresh Windows and Ubuntu machines**

- Windows: Null starts on a fresh Windows (it needed Microsoft's Visual C++ runtime,
  VCRUNTIME140.dll, which only comes with other software).
- Linux: with no file dialog on the system (no desktop portal, as on a bare window manager),
  Open, Open Folder and Add Folder let you type the path, listing what's in each folder.
- `null --version` and `null --help` answer in the terminal; on Linux without a display,
  Null says so instead of waiting.
- Typing `cart` in Ctrl+P finds cart.rs before Cargo.toml: letters typed together come
  first.
- Ctrl+P puts a source file before what's built from it: test_cart.py before
  __pycache__/test_cart.cpython-312.pyc.
- Running the test at the cursor (and a file's, or all of them) uses unittest, which comes
  with Python, when the project shows no sign of pytest; each test's result is read from it.
- Windows and Linux: Ctrl+Shift shortcuts (Ctrl+Shift+G the changes, Ctrl+Shift+P commands)
  work from the terminal too; plain Ctrl keys stay the shell's.
- Ctrl+K: a theme typed by its name ("paper") is offered as itself, not the row that steps
  through themes.
- Go to Line leaves the keys in the code, even when a list (the Git panel's branches) had
  them before.
- With no folder open, the status bar doesn't show Null's own scratch folder.
- Linux: keys the desktop doesn't take for itself. The test at the cursor is
  Ctrl+Shift+F10 (Ctrl+Alt+T opened a terminal), a cursor above or below Ctrl+Shift+↑/↓
  (Ctrl+Alt+↑/↓ switched workspaces).
- Windows and Linux: Ctrl+P and Ctrl+N in the search results and the suggestion list are
  Go to File and a new file again (they moved through the list, as on the Mac).
- The welcome keeps the keys while it shows: typing could reach a file reopened behind it.
- Linux: a project's window reopened at start is drawn at once (over remote desktop on
  GNOME it stayed blank until a key was pressed).
- Ctrl+K offers to install the file's missing language server ("Install Python Language
  Server"), as the status bar does.
- The Windows installer doesn't ask about an existing folder when installing again.
- After Close Folder the title and the files say no folder is open.
- AI is off until you turn it on: the welcome's AI step starts on Off (if you already use
  AI, it stays on).
- Linux: an API key is saved even where the system has no keyring yet (as after an xrdp
  login): Null asks for one to be made. The key can also come from its environment
  variable, as `NVIDIA_API_KEY`.
- A hosted AI service that hasn't started answering after a minute is said as such (with a
  hint to try another model), instead of "Writing…" forever: NVIDIA's free models can keep a
  request waiting for minutes.
- NVIDIA: the suggested models are ones its free tier answers today: Nemotron 3 Super, Nemotron
  3.5 Lightning (also for suggestions while typing) and GLM 5.3. DeepSeek V4.1 Flash never
  answered and the code models were gone; a saved one gives way to the default.

**Installers**

- Windows: an installer, for you alone (no administrator): the Start menu, `null` on the
  PATH, "Open with Null" on folders and files, an uninstaller. The .zip stays.
- Ubuntu and Debian: a .deb, `null` in /usr/bin for every terminal at once.
- Linux's install.sh adds ~/.local/bin to your shell's PATH when it isn't there (zsh never
  had it).
- Uninstalling, everywhere: the Windows uninstaller and Linux's new uninstall.sh (kept in
  ~/.local/share/null) ask whether your settings, saved API keys, history and language
  servers go too; on the Mac, ⌘K › Uninstall Null… quits, moves Null to the Trash and removes
  the `null` command. The guide has a section on it.

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
