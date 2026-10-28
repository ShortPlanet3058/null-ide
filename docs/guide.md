# The Null guide

Everything Null does, and how to reach it. Shortcuts are written as on the Mac; on Windows
and Linux, ⌘ is Ctrl, ⌥ is Alt and ⌃ is Ctrl, and a few differ to keep out of the way of
the system's own (Find Next is F3, tabs are Ctrl+PgUp / Ctrl+PgDn, bookmarks Ctrl+Alt+L /
Ctrl+Alt+J, folding Ctrl+Shift+[ and ], the terminal Ctrl+J, Join Lines Ctrl+Shift+J).
Shortcuts on digits and symbols follow your keyboard: on Windows they're on the key in the
same place as on an American keyboard (on a French AZERTY, the terminal is also Ctrl+ù,
comments Ctrl+:, tab 1 Ctrl+&), and on Linux an AZERTY's digit row works without Shift.
On Linux, where the desktop keeps Ctrl+Alt+T and Ctrl+Alt+arrows for itself, the test at
the cursor is Ctrl+Shift+F10 (debugged: Ctrl+Shift+F9) and a cursor above or below is
Ctrl+Shift+↑/↓; back and forward are Alt+←/→ everywhere off the Mac.
Menus and ⌘K always show the keys as they are on your computer, and for the keymap you
picked.

## Getting around

Settings open with ⌘, / Ctrl+,. They're saved in `~/.config/null/settings.json`
(`%APPDATA%\Null\settings.json` on Windows), which can also be edited by hand
(“Edit as JSON…” in Settings).

| Shortcut | Does |
|---|---|
| ⌘P | go to a file, recent ones first |
| ⌘K | quick settings (theme, text size, line spacing, wrap, sidebar, terminal, AI…) changed right in the list, and every command once you type |
| ⌘K "New Theme of Your Own…" | a theme file beside your settings (`themes/My Theme.json`), the one shown now written out: change a colour, save, and it shows; your themes are listed with Null's |
| ⌘, | all settings |
| ⌃G | go to a line |
| ⌘⇧O / ⌘T | go to a function or type, in the file / in the project |
| the status bar | after the file's name, where the caret is in the code: `shop.py › Shop › checkout` |
| the arrow at the end of the open file's row (or Show Outline) | its functions and types under it in the files, a method inside its class (in Markdown, its headings); the one you're in is marked, a click goes there |
| **Search in files** (above the files) | the project searched, the results in the files' place; Esc empties it, then goes back to the files |
| ⌘P `file:42:7` | open a file at a place, as compilers print it |
| ⌘F | find in the file (with lines selected, only in them); ↑ ↓ bring back earlier searches, ⌥↵ puts a cursor on every match; ⌘E searches for the selection, ⌘G the next |
| ⌘⇧F / ⌘⇧H | search / replace across the project (the chevron also picks which files: `*.rs, src/, !tests`; **Tabs**, only the files open) |
| replace in lower case | each match keeps its case: `user` → `client` makes `User` `Client` and `USER` `CLIENT` (typed with capitals, it goes in as typed) |
| ⌃- / ⌃⇧- | back / forward to where you were; ⇧⌘⌫ to the last edit |
| ⌃Tab | the tab used before this one; hold ⌃ and press Tab again to go further back (⌃⇧Tab the other way); ⌘⇧] / ⌘⇧[ go by position |
| ⌃⌘→ / ⌃⌘← | move the tab to the right or left side (split view) |
| right-click a tab → Move Below | the two sides one above the other (dragged, a tab offers "Open below" too); the line between them resizes |
| one click on a file | it opens in a preview tab (its name slanted), which the next file clicked replaces; edit it or double-click it (or its tab) to keep it (Settings → Preview tabs) |
| right-click a tab → Pin Tab | it stays first, out of "Close Others" and "Close All"; its pin unpins it |
| ⌥-click a file (or its menu → Open to the Side) | it opens in the other half of the window, split if it isn't |
| typing in the files | goes to the first file or folder whose name starts so, also one inside a closed folder (opened to show it) |
| ⌘⇧N / ⌥⌘O | a new window / a recent project |
| drop from the Finder | files open; on a folder in the files, they're copied there; on the terminal, their paths are typed in |
| ⌥⌘↵ | focus mode: only the code |
| ⌥⌘I | an AI task, reviewed change by change (with Claude Code or Codex) |

Coming from another editor? Pick its shortcuts (VS Code, JetBrains, Sublime Text or Zed) on
the welcome screen or in Settings → Keyboard. There, a click on a command's keys and then
the new ones gives it yours (Esc keeps them, ⌫ takes them away). They're written in
settings.json, over the preset's, where you can also write them:

```json
"keys": { "ctrl-cmd-l": "select all occurrences", "cmd-d": null }
```

A command is named as ⌘K lists it (or as in the code, `editor::SelectAllOccurrences`), and
`null` takes a shortcut away, everywhere in the window (the terminal and the files too).
Menus and ⌘K show your keys; one that can't be understood, or that would type a character
(`x`, a space), is named when settings.json is saved.

## Several folders in one window

A window can hold several folders, each a heading in the files with its own under it:
**Add Folder to Window…** (⌘K or File) picks one or more, a folder dropped from the Finder or
Explorer onto the files asks whether to add it or copy it in, and `null api web` opens them
together. A heading's menu has **Remove Folder from Window** (nothing on disk changes).

From a terminal, `null a.rs b.rs:42` opens each file (the first in front), in the folder
you're in, or in their repository when that folder is your home folder, `/`, or above it.
`null --help` lists the rest; `null -- -notes.md` opens a file whose name starts with `-`.

Across them all: ⌘P lists every folder's files (`web/app/page.tsx`), project search searches
them, ⌘T knows their symbols, and the files show git's marks. Each folder gets its own
language servers. Git follows the file you're in: the Git panel shows that folder's
repository, with a picker of the others at its top, and the status bar its branch. A new
terminal, ⌘⇧B's tasks, F5 and AI tasks work in the folder of the file at hand; tests are found
in every folder and run in their own.

The window remembers its folders: open its first one again (or pick it in Recent) and the
others come back with it.

Files show their language's logo (Rust's, Python's…) in the files, ⌘P and their tabs, in
one quiet colour; other files keep a dot. The caret glides to where it goes and the text
eases when it scrolls: View → Stop Smooth Caret and Scrolling (or ⌘K, or Settings → Cursor)
makes both jump instead.

## Writing

- **⌘.** offers the language server's quick fixes at the caret (a missing import, match
  arms…), and with AI on, Fix with AI for an error. **Organise Imports** (⌘K) has the server
  sort the file's imports and drop the unused ones, where it can (TypeScript, Go, Python). **F8** goes from problem to problem.
- Blocks fold from the gutter's arrow or ⌥⌘← / ⌥⌘→ (their first and closing lines stay);
  **Fold Level 1/2/3** (⌘K) folds every block that deep, for the file's outline.
- Multiple cursors (⌘D, ⌃⌘D to step over the one just picked, ⌘⇧-click), a box with ⌥⇧-drag, cursors at line ends (⌥⇧I);
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
- **The same change elsewhere**: type over a word (rename `count` to `total` by hand, or
  paste a word over it) and once you stop, the other places in the file the old word stands
  get a faint dot beside their line, and the line a faint "⇥ same change in 3 more places".
  ⇥ from the word changes the next one (each its own undo), and the next; Esc lets them be,
  and any other edit forgets them. (⌘R renames by meaning, with a language server.)
- **Bookmarks**: ⌘F2 marks the caret's line (its number turns amber), F3 and ⇧F3 go round
  them (Ctrl+Alt+L and Ctrl+Alt+J off the Mac), and **Bookmarks…** (⌘K) lists them in every open file. They move with the code and
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
  ⌃⌥← → go by the parts of a name (`parse` `Http` `Request`), ⌃⌥⇧ selecting, ⌃⌥⌫ and ⌃⌥⌦ deleting.
- Misspelled words in Markdown, text and comments get a faint wavy line, checked by the
  system's own speller: the Mac's (the languages set in System Settings), Windows' (your
  language and English, when Windows has them), and on Linux the Hunspell dictionaries
  installed (`hunspell-fr`, `hunspell-en-us`…; your language and English). Each line is
  checked in the one it reads best in, so French and English mix; code is never marked.
  **⌘.** on one offers corrections, or learns the word (on Linux, in Null's `words.txt`).
- Colours written in CSS, HTML, scripts and theme files (`#f80`, `rgb()`, `hsl()`) show a
  small square of themselves just before; on the Mac, a click on it opens its colour panel, and the
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
- Faint indent guides (the caret's block's a little brighter), bracket pairs in colour if you like (Settings, or ⌘K "Colour Bracket Pairs"), sticky scroll (the enclosing lines stay at the top), the other uses
  of a name tinted, and a line at the length the project keeps to (from `.editorconfig`,
  rustfmt, Prettier, Black or Ruff). Each can be turned off. Wrapped lines can break at that
  line rather than at the window's edge (Settings → Editor → Wrap at the line guide).
- Files keep their own style: indentation, line endings, and encoding (UTF-8 with or
  without BOM, UTF-16, Windows-1252), shown in the status bar when not the usual. A new
  file indents as its language is set to in settings.json: `"languages": { "Go": {
  "indent_with_tabs": true }, "Python": { "word_wrap": true } }` (also `indent_size`,
  `format_on_save`, `format_on_paste`, `autocomplete`). One read
  wrong (garbled accents, or "isn't text") is read again with ⌘K "Encoding: Reopen as…";
  "Encoding: Save as…" writes it in another.
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
  (onto Markdown, a link each). The question before the Trash has "Don't ask again" (space
  ticks it); Settings → Window → Ask before moving to the Trash brings it back.
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

- **⌃⇧G** opens the Git panel at what changed since the last commit; each file opens with
  its changes to keep or discard one by one.
- **Commit…** puts the keyboard in the panel's message: tick files (or parts of them) in or
  out; with AI ready, **Draft** writes the message from the changes. **Undo Last Commit**
  takes it back (not once pushed), its message back in the field for the next. **Set Changes Aside** and **Bring Back Changes** stash and
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
- Merge conflicts: each side tinted, **⌘.** keeps one or both, **⌥F8** to the next. With AI
  on, **✦ Resolve with AI** in the same list writes one version keeping what each side
  changed, shown as a change: ⇥ keeps it, Esc undoes it.

## Running, testing, debugging

- **⌘⇧B** runs what the project defines (Cargo, package.json scripts, Make, just, Go, the
  tasks of a `.vscode/tasks.json`), and your own: `"tasks": { "deploy": "./deploy --prod" }`
  in settings.json, in every project.
- **⌥⌘T** runs the test at the caret (Rust, Go, pytest, vitest, jest), or all of a file's.
- The terminal (**⌃\`**, more with **⌃⇧\`**, two side by side with **⌘D** in it; a double-click on one's name renames it, and a
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
  With AI on: type `# what you want` at the prompt and press **Tab**, and the command for it
  takes its place on the line, for you to read and run with Return (never run for you). When
  what a command printed says it went wrong, **Explain what went wrong** shows in the corner
  (also in ⌘K): what happened, and the command that puts it right, to put on the line.
- **F5** debugs (lldb-dap: with Xcode on the Mac, LLVM on Windows, LLDB on Linux): breakpoints with **F9**, conditions with a
  right-click, values shown faintly beside the code and in a panel. Under the variables,
  type an expression to watch (`letters * 2`): its value shows at every stop.


## Sessions

Null reopens each project as you left it: the same tabs, each with its caret and scroll,
the folders open in the tree, the terminal, and the window's place. Opened from the Finder
or the Dock, it comes back to the last project. Sessions are small files in Null's own folder
(`~/Library/Application Support/Null/sessions` on macOS, `%LOCALAPPDATA%\Null\sessions` on
Windows, `~/.local/share/null/sessions` on Linux; `NULL_DATA_DIR` overrides it).

## Folders inside WSL (Windows)

**Open WSL Folder…** (in ⌘K or the File menu) opens a folder inside WSL, starting from your
Linux home; `null .` typed in a WSL shell does the same (after Install Shell Command). Null
reads and writes the files through Windows' WSL share, and runs everything else inside Linux,
in the folder's Linux path: the terminal is your Linux shell, git is Linux's (its settings and
credentials), and language servers run and install there (into `~/.local/share/null`, with
Node.js when the distribution has none). Tasks and tests are written for that shell. Changes
made in WSL by other programs are noticed within a couple of seconds.

On WSL 1, servers built on Node.js (Python's, TypeScript's, and others) can't run: Node.js
doesn't start there. Move the distribution to WSL 2 (`wsl --set-version Ubuntu 2`, with its
name); the other servers work on both.

## Folders on servers (SSH)

**Open SSH Folder…** (in ⌘K or the File menu) asks for a server, as you'd give it to `ssh`
(`me@example.com`, a name from `~/.ssh/config`, `box#2222` for another port; those you've
used and those in your config are listed), connects, then lists your home folder there to
pick a folder from. `null ssh://me@example.com/home/me/app` opens one from a terminal (with
`/~/app` for one in your home folder). Open Recent lists them too.

Null uses your own `ssh`: your keys, your agent and your `~/.ssh/config`. Nothing is
installed on the server; it needs a shell and the usual tools (Linux, or a Mac). Files are
read and written through one connection that stays open; the terminal, git and language
servers run on the server, in the folder (language servers install there, into
`~/.local/share/null`, with Node.js when the server has none). Saving writes the whole file
beside the old one, then swaps it in; Move to Trash uses the server's Trash
(`~/.local/share/Trash`). Changes made on the server by other programs are noticed within a
few seconds. Search looks through the files on the server first, and reads only those that
can match.

Your keys and agent are used as `ssh` uses them. When `ssh` asks something (a password, a
key's passphrase, whether to trust a server it hasn't met), Null asks it in the window; an
answer is kept in memory until Null quits, so the terminal's `ssh` to that server doesn't
ask again. On a Mac and Linux, Null shares one connection to each server between
everything it runs there, so programs start at once.

The project's Python environment, the Tests panel and the debugger work there as they do
here. File History keeps a server file's earlier versions on this computer, from what Null
last read or saved of it. Pictures open from the server; pictures in Markdown on a server
show their description, as pictures from the web do.

## New versions

Null doesn't look for updates unless asked: turn on "Tell me when a new version is out" (on
the welcome's last step, or Settings → Appearance) and once a day it reads the latest
release's details from GitHub; "Check for Updates" in ⌘K asks right away. A newer one shows
as "Null 0.2.0 is out · Update" in the status bar.

Nothing is installed until you click Update. Null then downloads the package for how you
installed it, checks it's the release (its details are signed with Null's own key, and the
package must have the digest they give), and says "Restart to Update": Null quits as usual
(asking about unsaved changes), the new version is put in place, and it starts again. Not
restarted, it's installed the next time Null quits.

- Windows (installed with setup.exe): the installer runs again, quietly, with your choices.
- Mac: the app is replaced where it is.
- Linux: the binary (`~/.local/bin/null`, or where you put the .tar.gz's) is replaced; a
  .deb is installed again, the system asking for your password.

The Windows zip, a build from source, or a folder Null can't write to: "Null 0.2.0 is out"
opens null-ide.com instead.

After an update, a notice says which version is installed; a click on it shows what changed
since the version you had. "What's New" in ⌘K shows it again.

## Code intelligence

Completions, errors, info on hover and go to definition come from language servers. Null
knows which one each language needs, finds it if it's installed (Xcode's clangd and
sourcekit-lsp included), and otherwise offers to install it with one click, from the status
bar or Settings → Language servers, into its own folder rather than the system.

Nothing else needs installing first: servers that are npm packages bring their own Node.js
when the computer has none (the official build, downloaded into Null's folder and used only
for them), and servers that are programs of their own are downloaded from their makers'
releases. Every download is checked against the checksum Null was built with.

| Language | Server | Installed with |
|---|---|---|
| Rust | rust-analyzer | rustup when you have it, else downloaded |
| Python | pyright | npm |
| TypeScript, JavaScript | typescript-language-server | npm |
| C, C++, Objective-C | clangd | Xcode's command line tools on macOS, else downloaded |
| Go | gopls | go (needs Go) |
| Swift | sourcekit-lsp | comes with Xcode |
| Shell | bash-language-server | npm |
| HTML, CSS, JSON | vscode-langservers-extracted | npm |
| YAML | yaml-language-server | npm |
| Markdown | marksman | downloaded |
| Lua | lua-language-server | downloaded |
| PHP | intelephense | npm |
| Svelte | svelte-language-server | npm |
| Dockerfile | dockerfile-language-server | npm |
| C# | csharp-ls | dotnet (needs the .NET SDK) |
| Dart | the Dart SDK's own | comes with the Dart SDK |

With a server running: ⌘R (or F2) renames a symbol everywhere, right where it's written;
⇧F12 lists where it's used (⌘-clicking a definition does too), ⌃⌥H where a function is
called from (and ⌘K "Show What This Calls" what it calls), ⌥F12 shows a definition where you are without going there; ⌥⇧F formats the file, and
"Format on save" does it on ⌘S (and "Format on paste", to pasted code, where the server formats
parts of files); the caret's line says its problem at its end ("Problems at Line Ends" in
⌘K or Settings: every line's, faintly); ⌘⇧M, or the error count in the status bar, lists every
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

- **⌘I** opens a field between your lines. Describe a small change (or press Enter
  on an error to fix it): it's written into the file as a diff, with removed lines struck
  through and new ones tinted. ⇥ keeps it, Esc undoes it, ⌘I adjusts it. ⇧↵ starts another
  line in the field (pasted text keeps its lines); so do a follow-up question's field and an
  AI task's (⌥⌘I).
- **What ⌘I changes** is tinted while the field is open: the lines selected, else the
  function, class, CSS rule or HTML element the cursor is in (with its doc comments and
  attributes), else the paragraph around it. Without a selection it's never more than 40
  lines: in a bigger one, it's the part the cursor is in (an `if`, a `<section>`), or its line.
- **Questions** (typed in ⌘I, or “Ask About This File” in ⌘K) get a short note under the code
  they're about. Esc closes it.
- **⌘I writes in place**: the new code appears line by line as it's written, over the
  dimmed code it replaces.
- **Suggestions while typing** (Settings → AI, off by default): names from the file appear
  the moment you type two letters, then the AI's guess when you pause, streamed in. ⇥ takes
  it, ⌥→ takes the next word, ⌘→ the rest of the line, ⌥⇥ shows another option; typing along
  keeps it, and deleting back brings an earlier one straight back. It knows what the whole
  project defines (a local index, no AI) and the other open files. They come from a code model made to fill in the middle
  (Codestral on Mistral, Qwen2.5-Coder on Ollama; changeable in Settings → AI), which sees the
  file and the files it includes or imports, and can write a whole body after `{` or `:`.
  They can come from another provider than answers (Settings → AI → Suggestions come from).
  Claude Code and Codex take seconds to answer, too slow between two keystrokes: with them,
  Null asks once where suggestions should come from (Mistral's free Codestral, Ollama on
  your computer, another key you have, or names from the file only).
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

**Notes on code.** ⌥⌘N (Ctrl+Alt+N) writes a note on the cursor's line: it sits quietly
under the line and moves with it as you edit; click it to change it, empty it to remove it.
Notes are yours: kept in Null's own folder (never in the project, nothing to commit by
accident), and they find their line again when the file changed elsewhere. **Notes…** in ⌘K
lists every note in the project. An answer from ⌘I can be kept as a note too ("Keep as note"),
and asked about further ("Ask a follow-up", which goes with the questions before it).

**Review before committing.** **Review** in the Git panel has AI read the selected changes and
leave a note on each line it has doubts about (a bug, a missed case), marked "AI"; it never
changes the code. Reviewing again replaces those notes. After a test fails, **Explain Test
Failure** in ⌘K opens the failed test with a note on why, from what its run said.

**What the AI reads.** With ⌘I, only what the request needs: the file when it's short, or in
a long one the lines around the code with an outline of the rest; the definitions that code
uses from elsewhere in the project (from Null's own index of the project, no language server
needed); and the project's conventions (`AGENTS.md`, `CLAUDE.md`, `.github/copilot-instructions.md`
or `CONVENTIONS.md`, for providers other than Claude Code and Codex, which read them
themselves). The field says what goes with the request and about how many tokens. What stays
the same from one request to the next is sent first, so providers can take it from their
cache (Anthropic's is asked for; OpenAI, Gemini, DeepSeek and others do it by themselves), and
an AI task's conversation is cached as it grows, so each step pays only for what's new.

**Tokens.** Settings → AI → AI usage shows this month's tokens (and how many came from a
cache); "AI Usage" in ⌘K says the last request's too. A monthly limit (1M, 5M or 20M tokens)
stops every AI request, suggestions included, once it's reached, until the next month. The
counts stay on your computer (`ai-usage.json` in Null's folder).

API keys are read from the provider's usual variable (`MISTRAL_API_KEY`, `GROQ_API_KEY`,
`GEMINI_API_KEY`, `OPENROUTER_API_KEY`, `NVIDIA_API_KEY`, `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`) when set,
otherwise from the system's key store (the keychain, Windows Credential Manager, the Secret Service) (Settings → AI → API key), at most once per launch; they
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

## Uninstalling

Each way of removing Null asks whether your settings, saved API keys, history and language
servers go too; by default they stay, for if you come back.

- **Mac**: ⌘K › Uninstall Null… (also in the Null menu): Null quits, goes to the Trash, and
  takes the `null` command with it. Dragged to the Trash instead, it leaves those behind.
- **Windows**: Settings › Apps › Installed apps › Null › Uninstall.
- **Linux**, from the `.deb`: `sudo apt remove null-editor` (your settings stay); from the
  `.tar.gz`: `~/.local/share/null/uninstall.sh`.

Where things are kept: settings in `~/.config/null` (`%APPDATA%\Null` on Windows); history,
sessions and language servers in `~/Library/Application Support/Null` on the Mac,
`~/.local/share/null` on Linux, `%LOCALAPPDATA%\Null` on Windows; API keys in the system's
keychain.
