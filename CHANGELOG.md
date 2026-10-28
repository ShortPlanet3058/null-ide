# Changelog

## Unreleased

- A click on who last changed the line (at its end, with Show Who Changed the Line on)
  opens that commit.
- Settings → AI with Ollama lists the models it has installed, first, to pick with a click.
- Files that hold secrets (`.env`, private keys, `.npmrc`…) are never sent to AI: ⌘I says so
  there, suggestions stay quiet, and Review and commit drafts leave them out.
- Debugging: a variable or watch whose value changed since the last stop shows in amber.
- Expand and Shrink Selection (⌥↑ ⌥↓) work on every cursor, each by its own syntax.
- More from the outside audit:
  - The commit message's body field takes several lines and keeps them (Undo Last Commit
    gave back a body flattened onto one line).
  - ⌘K: typing a group's name (`git`) lists its commands before others whose name only has
    its letters strewn across it ("Go to Last Edit").
  - SSH: a mistyped password is asked again, not tried three times over (a server can lock
    you out for that); a new file on a server is made only if none has that name by then.
  - An AI task's review takes in a file that grew past 1 MB or became ignored while it ran.
  - The debugger ends a program it started before letting go of the adapter, and pressing
    Stop while it read where the program stopped no longer leaves it "stopped" with nothing.
  - Language servers, debuggers and AI tasks stopped are waited for (no leftover
    processes); closing a window saves its session as it is and stops its servers.
  - Notebooks: a Markdown cell ending in a line break shows no extra empty line.
  - Unsaved work kept for after a crash: an older background write can't replace a newer
    one, and the last run's backups are removed only once kept in this run's.
  - Switching branch with changes that can't be put back on the other says they're set
    aside (Bring Back Changes); Revert on a server keeps the file's history.
  - Linux's install.sh writes the menu entry right for a home folder with a space or `&`.

## 0.2.1

Null tells you what's new after it updates itself, picks what ⌘I changes from the code's
structure, takes several lines in its AI fields, helps with merge conflicts, checks
spelling on Windows and Linux too, and fixes what an outside audit found.

- What's new: after an update, a notice says which version is installed, and a click on it
  shows the changes since the version before (What's New in ⌘K, any time).
- ⌘I without a selection changes the function, class, CSS rule or HTML element the cursor is
  in, in every language with one (it was only Rust's; elsewhere the lines up to a blank one,
  so a whole HTML page), and never more than 40 lines: in a bigger one, the part the cursor
  is in.
- AI fields take several lines: ⇧↵ starts one in ⌘I, a follow-up question and an AI task
  (⌥⌘I), and pasted text keeps its lines; the field grows up to 8 rows, then scrolls.
- Merge conflicts: ✦ Resolve with AI (⌘. in a conflict, with AI on) writes one version
  keeping what each side changed, shown as a change to keep (⇥) or undo (Esc).
- The same change elsewhere: a word typed over (a name renamed by hand) marks the other
  places in the file the old word stands; ⇥ from the word changes them one by one, Esc
  lets them be.
- Spelling on Windows and Linux: Windows' own speller, and on Linux the Hunspell
  dictionaries installed, in your language and English, each line in the one it reads best
  in (it was the Mac's only).
- Moving files to the Trash asks in the window, with "Don't ask again" (space ticks it);
  Settings → Window → Ask before moving to the Trash brings the question back.
- Fixes from an outside audit:
  - Markdown no longer crashes on a list line starting with a wide space (Japanese text),
    or on ⇥ in a table row holding a lone backtick.
  - The cursor can't end up past the end of the file (after a review's last change was
    undone, or a late completion), which crashed Run Test at Cursor.
  - ⌘I's adjust asks about the lines first changed again, not as many as the change made
    (it reached the code below a change that had grown).
  - A rename or quick fix from a language server isn't applied if the file was typed into
    while it was worked out (its places had moved): Null says to try again.
  - An AI task's change undone in review is undone on disk too; a file it added, undone
    entirely, goes to the Trash.
  - Delete Line on the last line of a Windows (CRLF) file takes its whole line break.
  - Over SSH, a save that fails (full disk) never leaves the file half written, and a
    read-only file isn't written over. In a WSL folder, renaming a file to another one's
    name in other case no longer replaces it.
  - Unsaved changes brought back after a crash remember that the file changed on disk
    meanwhile, so saving still asks first.
  - The crash log, trimmed past 1 MB, could crash Null again while writing it.
  - New File and New Folder in Project from ⌘K open the name field in the files (they did
    nothing); a named pipe, socket or device isn't opened (a pipe hung the window).
  - Opening another project in the window (Open Recent) takes its settings at once, closes
    the last one's terminals (asking first if a command runs there), and stops what ran
    for it (the debugger, an AI task).
  - Command line: every file opens (`null a.rs b.rs`), options anywhere (`null a.rs
    --wait`), `--` before a file named with a `-`, and an unknown option is said instead
    of opened as a file. A file asked from your home folder or `/` opens its repository,
    not the whole folder. On Linux, a display that's gone (a stale DISPLAY) is said in the
    terminal instead of crashing.
  - Folders on servers: ⌘P opens the file picked (it looked for it on this computer and
    took it off the list instead), accented file names (`café.rs`) open from ⌘P and search, a folder
    named with `&` lists its files, a new file opens once made, pasting copied files works,
    ⌘⇧B finds the project's tasks, and the GitHub tab (which runs here) isn't offered.
    Merge conflicts on servers and in WSL can be finished from Null.
  - Vim: `.` after a line started with `}` types it again (it typed from the wrong place),
    and `dd` on a new empty last line takes that line, not the one above.
  - Null's data folder (backups of unsaved work, sessions, history) is its owner's only;
    the SSH connection folder in /tmp is used only if it's yours; a lock file no longer
    piles up each time Null runs; settings that can't be read no longer bring the welcome
    back at every start.

## 0.2.0

Null updates itself, opens folders inside WSL and on servers over SSH, installs language
servers on a computer with nothing else installed, and asks AI for less.

- Folders on servers: File History keeps a server file's earlier versions (from what Null
  last read or saved of it, no extra trip to the server), and pictures open from there.
  F5 follows a server project's .vscode/launch.json (it wasn't read there), with lldb-dap
  on the server.
- Relative line numbers (Settings → Editor): lines counted from the cursor's, as a count
  before a move goes (Vim's 5j); a fold counts as one line.
- Update: a newer version out shows as "Null 0.2.0 is out · Update" in the status bar; a
  click downloads it, checks it's the release (signed details, and the package's SHA-256),
  then Restart to Update puts it in place and starts Null again (or it's installed when Null
  quits). Windows' installer, the Mac app, Linux's binary or .deb; elsewhere null-ide.com.
- Windows: AltGr and dead keys type in the terminal (an AZERTY's ~ # \\ { [ ê), and no
  shortcut takes a key AltGr types with (Ctrl+Alt+0 and Ctrl+Alt+= kept @ and } from
  being typed on an AZERTY; Reset Zoom and Zoom In stay in the View menu there).
- Opening a server's folder from Open Recent connects first instead of freezing, and Open
  Recent lists each folder once.
- The sidebar is quieter: Files, Tests and Git. Search is a field above the files (⌘⇧F),
  its results in their place; the outline opens under the open file, from the arrow at the
  end of its row or Show Outline.
- SSH: passwords, passphrases and trusting a new server asked in the window (Windows too);
  git through the open connection; a server's folder reopened at launch once reachable;
  Python environments, the Tests panel and the debugger on servers.
- Tests: results read where the terminal can't see its programs (servers, WSL, Windows);
  unittest run on a `tests/` folder that isn't a package finds its tests.
- Files show their language's logo where they had a dot (the files, ⌘P) and in their tabs,
  in the dot's one colour.
- Open to the Side: ⌥-click a file, or its menu, opens it in the other half of the window.
- Typing in the files also finds a file inside a closed folder, and opens the folders to it.
- Smooth caret and scrolling can be turned off (View menu, ⌘K, Settings → Cursor).
- Language servers install without npm's update check.
- Folders on servers over SSH: Open SSH Folder… (a server, then a folder from its home), or
  `null ssh://me@box/path`. With your own ssh, keys and config, nothing installed there:
  files through one connection, the terminal, git and language servers run on the server,
  saves are whole, Move to Trash uses the server's Trash, changes there show within seconds,
  search narrows files on the server first.
- CI runs once a pull request is ready, not on drafts or after merges; packages are built
  when a new version reaches main, or on demand, per platform.
- Folders inside WSL (Windows): Open WSL Folder…, or `null .` in a WSL shell. Files are read
  through Windows' WSL share; the terminal, git and language servers run inside Linux, in the
  folder's Linux path, and servers install there (Node.js along when the distribution has
  none). Changes made there are looked for every couple of seconds. On WSL 1, servers built
  on Node.js say they need WSL 2 (Node.js doesn't start there).
- Windows: while a question is open ("Save changes?"), the window behind it takes no keys or
  clicks; `null --version` and `null --help` print in the console they're typed in.
- "Tell me when a new version is out" (off until turned on, on the welcome's last step or in
  Settings): once a day Null asks GitHub for its latest release, and says so in the status
  bar when there's a newer one. "Check for Updates" in ⌘K asks now.
- Notes on code: ⌥⌘N (Ctrl+Alt+N) writes a note under the cursor's line; it moves with its
  line and is kept privately in Null's folder, never in the project. Notes… lists them.
- An answer from ⌘I can be asked about further (each follow-up goes with the questions
  before it) and kept as a note on the code.
- Review in the Git panel: AI reads the changes about to be committed and leaves a note on
  each line it has doubts about, never a change. Explain Test Failure (⌘K) explains the last
  failed test from what its run said.
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
