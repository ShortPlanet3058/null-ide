# Changelog

## 0.1.1

What the first tests on Windows and Linux found, several folders in one window, and a new
license.

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
- After Close Folder the title and the files say no folder is open.
- AI is off until you turn it on: the welcome's AI step starts on Off (if you already use
  AI, it stays on).
- Linux: an API key is saved even where the system has no keyring yet (as after an xrdp
  login): Null asks for one to be made. The key can also come from its environment
  variable, as `NVIDIA_API_KEY`.

**Installers**

- Windows: an installer, for you alone (no administrator): the Start menu, `null` on the
  PATH, "Open with Null" on folders and files, an uninstaller. The .zip stays.
- Ubuntu and Debian: a .deb, `null` in /usr/bin for every terminal at once.
- Linux's install.sh adds ~/.local/bin to your shell's PATH when it isn't there (zsh never
  had it).

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
