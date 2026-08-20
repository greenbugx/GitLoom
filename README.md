# GitLoom

A fast, lightweight, keyboard-first terminal-based Git history explorer designed to make complex Git repositories easy to understand.

## Features

- **Native terminal UI** built with `ratatui` and `crossterm`.
- **Beautiful graph rendering** with per-lane colored glyphs, diagonal and corner connectors for branches and merges, all drawn in a strict 1-row-per-commit layout.
- **Keyboard-driven navigation** across commits, views, and search results.
- **Inline ref badges** next to each commit summary — branches, remote branches, and tags color-coded at a glance.
- **Conventional-commit prefix coloring** for `feat`, `fix`, `docs`, `chore`, `refactor`, and `test` commits.
- **Commit activity minimap** — a per-row sparkline derived from each commit's insertions and deletions.
- **Detailed views** for commit details, changed files, diffs, and branches & tags.
- **Incremental search** over commit summaries, authors, and OIDs.

## Usage

```bash
cargo run [path-to-repo]
```

If no path is given, GitLoom opens the current directory.

### Keybindings

| Key | Action |
| --- | --- |
| `↑`/`↓`, `j`/`k` | Navigate commits / scroll details |
| `Enter` | Show commit details |
| `f` | Show changed files |
| `d` | Show diff |
| `b` | Show branches & tags |
| `/` | Search |
| `n`/`N` | Next / previous search match |
| `Esc` | Close current view |
| `q` | Quit |

## License
Check [MIT](LICENSE) for license info.