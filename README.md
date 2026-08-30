# GitLoom

<img width="130" height="105" alt="Screenshot 2026-08-24 005923" src="https://github.com/user-attachments/assets/457395cf-aa37-4b6f-8245-466b6b210d7f" />

A fast, lightweight, keyboard-first terminal-based Git history explorer designed to make complex Git repositories easy to understand.

## Features

- **Native terminal UI** built with `ratatui` and `crossterm`.
- **Animated Loading Screen** inspired by Loom, featuring a smooth and minimal loading experience.
- **Beautiful graph rendering** with per-lane colored glyphs, diagonal and corner connectors for branches and merges, all drawn in a strict 1-row-per-commit layout.
- **Keyboard-driven navigation** across commits, views, and search results, with a `?` overlay that documents every binding.
- **Inline ref badges** next to each commit summary — branches, remote branches, and tags color-coded at a glance.
- **Conventional-commit prefix coloring** for `feat`, `fix`, `docs`, `chore`, `refactor`, and `test` commits.
- **Commit activity minimap** — a per-row sparkline derived from each commit's insertions and deletions.
- **Detailed views** for commit details, changed files, diffs, and branches & tags, fetched on a background thread so a large commit never stalls the UI.
- **Single-path file history** — scope the graph to one file and walk only the commits that changed it.
- **Incremental search** over commit summaries, authors, and OIDs.
- **Copy a commit hash** with `y`, using OSC 52 so it works over SSH too.
- **Unbounded HEAD history** — commit history loads a page at a time and keeps loading automatically as you scroll, instead of stopping at a fixed commit ceiling.

## Screenshots

<img src="https://github.com/user-attachments/assets/5a46bc1f-7f00-49fc-ba6c-359f7179fa66" width="100%">
<img width="100%" alt="image" src="https://github.com/user-attachments/assets/2ea2257b-c419-471d-ade4-a2d7ee4adf37" />
<img width="100%" alt="image" src="https://github.com/user-attachments/assets/884bf2f6-f6c1-415b-bb65-b9b1e424f211" />
<img width="100%" alt="image" src="https://github.com/user-attachments/assets/90196e00-2643-4db8-8b43-161e74b5e044" />
<img width="100%" alt="image" src="https://github.com/user-attachments/assets/bf9d9211-55e0-4728-a081-a6e6ae8ca9a2" />

## Installation

### Prebuilt binaries

The [latest release](https://github.com/greenbugx/GitLoom/releases/latest) has
archives for Linux (x86_64), Windows (x86_64) and macOS (Apple Silicon and
Intel). Unpack one and put `gitloom` somewhere on your `PATH`:

```bash
tar xzf gitloom-v0.1.0-x86_64-unknown-linux-gnu.tar.gz
sudo install gitloom-v0.1.0-x86_64-unknown-linux-gnu/gitloom /usr/local/bin/
```

On Windows, unzip the archive and either add that folder to your `PATH` or run
`gitloom.exe` from where it landed.

The macOS binaries aren't signed or notarized, so the first run is blocked until
you clear the quarantine flag your browser attached to the download:

```bash
xattr -d com.apple.quarantine ./gitloom
```

Every release ships a `SHA256SUMS` file alongside the archives. Check it with
`sha256sum -c --ignore-missing SHA256SUMS` on Linux, `shasum -a 256` on macOS, or
`Get-FileHash <archive> -Algorithm SHA256` in PowerShell.

### From crates.io

```bash
cargo install gitloom-tui
```

Needs **Rust 1.88 or newer**: GitLoom is on edition 2024 and the event loop uses
let-chains. libgit2 is compiled as part of the build, so a working C compiler is required too but nothing else, since GitLoom pulls in no OpenSSL dependency.

### From source

```bash
git clone https://github.com/greenbugx/GitLoom.git
cd GitLoom
cargo install --path .
```

Or `cargo build --release` and run `target/release/gitloom` where it is.

## Usage

```bash
gitloom [path-to-repo]
```

If no path is given, GitLoom opens the current directory; any directory inside a
working tree will do. `--help` and `--version` print and exit without opening the
TUI.

Working on GitLoom itself, without installing it: `cargo run -- [path-to-repo]`.

### Keybindings

Press `?` inside GitLoom for this same table. `src/ui/help.rs` is the source of
truth for both.

| Key | Action |
| --- | --- |
| `j`/`k`, `↓`/`↑` | Down / up — moves the selection, or scrolls a pane |
| `J`/`K` | Next / previous commit, keeping the open pane in step |
| `g`/`G`, `Home`/`End` | Top / bottom of the focused pane |
| `PgUp`/`PgDn` | Move by a screenful |
| `h`/`l`, `←`/`→` | Scroll a wide pane left / right |
| `Enter` | Show commit details |
| `f` | Show changed files |
| `d` | Show diff |
| `b` | Show branches & tags |
| `l` in the files pane | History of the selected file |
| `/` | Search summaries, authors, and OIDs |
| `n`/`N` | Next / previous search match |
| `y` | Copy the selected commit's full hash |
| `?` | Toggle the keymap overlay |
| `Esc` | Close the current view, or leave a file's history |
| `q` | Quit |

## License
Check [MIT](LICENSE) for license info.
