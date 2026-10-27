# Working on Null

Null is written in Rust with [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui),
[tree-sitter](https://tree-sitter.github.io/), and the
[Language Server](https://microsoft.github.io/language-server-protocol/) and
[Debug Adapter](https://microsoft.github.io/debug-adapter-protocol/) protocols.

Null is proprietary software (see [LICENSE](LICENSE)). Contributions are welcome: by
sending one (a pull request, a patch), you grant the author the right to use, change,
sublicense and sell it as part of Null, as the license says.

## Building

You need [Rust](https://rustup.rs) (stable), and on macOS Xcode. On Linux, the libraries
GPUI draws with; on Debian or Ubuntu:

```sh
sudo apt install libfontconfig-dev libxkbcommon-x11-dev libx11-xcb-dev libwayland-dev \
    libvulkan-dev libasound2-dev libudev-dev libdbus-1-dev
```

```sh
cargo run                      # open the current folder
cargo run -- path/to/folder    # open another folder
cargo run -- path/to/file.rs   # open a file
cargo test                     # the tests (they never touch your files or settings)
```

Shaders are compiled when the app starts (GPUI's `runtime_shaders` feature), so the build
doesn't need Xcode's separate Metal Toolchain download.

## Packages

```sh
packaging/macos/bundle.sh             # target/release/Null.app, for this Mac
packaging/macos/bundle.sh --install   # and copies it to /Applications
packaging/macos/bundle.sh --release   # for every Mac (Intel and Apple silicon), in a .dmg
packaging/linux/install.sh            # Linux: built, into ~/.local/bin and the menu
packaging/linux/package.sh            # Linux: the release tarball
powershell -File packaging\windows\package.ps1   # Windows: the release zip
```

The Mac app is signed ad hoc unless `NULL_SIGN_IDENTITY` names a Developer ID.

## Checking other platforms from a Mac

`scripts/check-platform.sh windows` (or `linux`) type-checks Null for that platform without
a virtual machine; `NULL_CHECK=clippy scripts/check-platform.sh windows --all-targets -- -D
warnings` lints it. `scripts/check.sh` runs all of what CI checks, on the Mac (`--quick`
skips the tests). CI builds, lints and tests a pull request on macOS, Windows and Linux once
it's ready (drafts are checked locally); a package to try before merging is built on demand:
`gh workflow run release.yml --ref <branch> -f platform=windows` (or `linux`, `macos`, `all`).

Packages are built only then, and when a merge into `main` carries a version not released
yet (Cargo.toml's): that builds the three platforms and makes a draft release, tagged
`v<version>` when it's published. Merges into `dev` build no packages.

## Looking at it

`scripts/qa/shot.sh STEPS OUT.png [PROJECT]` runs a debug build on a copy of a project, plays
the steps in a file (open a file, type, press keys, run an action…, see `src/qa.rs`) and
takes pictures of its window, without touching your own settings or files.

## Branches

- `main`: what's released; only merges from `dev`.
- `dev`: where work comes together.
- `feature/<topic>`, `fix/<topic>`: from `dev`, with a pull request back into it.

## Releasing

Set the version in `Cargo.toml`, add its notes to `CHANGELOG.md`, and push a tag of it
(`v0.1.0`): CI builds the three packages and drafts a GitHub release to look over and publish.
