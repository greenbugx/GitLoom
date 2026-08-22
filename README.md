# GitLoom

A fast, lightweight, keyboard-first terminal-based Git history explorer designed to make complex Git repositories easy to understand.

## Features

- **Native terminal UI** built with `ratatui` and `crossterm`.
- **Animated Loading Screen** inspired by Loom, featuring a smooth and minimal loading experience.
- **Beautiful graph rendering** with per-lane colored glyphs, diagonal and corner connectors for branches and merges, all drawn in a strict 1-row-per-commit layout.
- **Keyboard-driven navigation** across commits, views, and search results.
- **Inline ref badges** next to each commit summary — branches, remote branches, and tags color-coded at a glance.
- **Conventional-commit prefix coloring** for `feat`, `fix`, `docs`, `chore`, `refactor`, and `test` commits.
- **Commit activity minimap** — a per-row sparkline derived from each commit's insertions and deletions.
- **Detailed views** for commit details, changed files, diffs, and branches & tags.
- **Incremental search** over commit summaries, authors, and OIDs.

## Screenshots

<div align="center">

<table>
<tr>
<td><img src="https://github.com/user-attachments/assets/5a46bc1f-7f00-49fc-ba6c-359f7179fa66" width="100%"></td>
<td><img src="https://github.com/user-attachments/assets/86e6d15e-e83a-4e32-be6a-c6b8fd68da17" width="100%"></td>
</tr>
<tr>
<td><img src="https://github.com/user-attachments/assets/d56a3d47-d848-4cd8-8802-caed6b1e3436" width="100%"></td>
<td><img src="https://github.com/user-attachments/assets/2fa94c69-d156-4264-9183-8c33af93a272" width="100%"></td>
</tr>
</table>

</div>


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
