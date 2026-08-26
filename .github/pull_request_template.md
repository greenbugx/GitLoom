<!-- Thanks for contributing to GitLoom! Fill in what applies and delete what
     doesn't — a short, accurate description is worth more than a complete but
     padded one. -->

## Summary

<!-- What does this change, and why? Lead with the behavior a user would
     notice; leave the implementation detail for the notes at the bottom. -->

## Related Issue

<!-- "Closes #12", or "Refs #12" if it only moves the issue along. Write "None"
     for unprompted changes. If part of the issue turns out to have been fixed
     already, name the commit that did it — otherwise a reviewer is left hunting
     the diff for changes that aren't in it. -->

Closes #

## Type of Change

<!-- Matching the Conventional Commit types listed in CONTRIBUTING.md. -->

- [ ] `feat` — new feature
- [ ] `fix` — bug fix
- [ ] `docs` — documentation only
- [ ] `refactor` — no change in behavior
- [ ] `test` — tests only
- [ ] `chore` / `ci` — tooling, dependencies, workflows
- [ ] Breaking change — an existing keybinding, CLI flag, or output changes meaning

## How This Was Tested

<!-- Much of a TUI can't be asserted in a test, so the manual half is the part
     a reviewer can't reconstruct. Be specific about what you actually did. -->

- **Automated:** <!-- e.g. `cargo test`; name any tests you added -->
- **By hand:** <!-- the keys you pressed and what you looked at -->
- **Tested on:** OS [e.g. Windows 11 / Ubuntu 24.04 / macOS 15], terminal [e.g. Windows Terminal / Alacritty / Kitty]

<!-- CI covers ubuntu-latest and windows-latest because the event loop differs
     between them: Windows reports a key press *and* a release per keystroke. If
     you touched key handling, say which platform you actually ran it on. -->

## Screenshots / Recordings

<!-- Worth including for anything that changes what gets drawn — lanes, ref
     badges, panes, the help overlay, the loading screen, colors. A before/after
     pair saves a reviewer from building the branch to see the difference. -->

## Checklist

- [ ] `cargo fmt` — CI runs `cargo fmt --check`
- [ ] `cargo clippy --all-targets -- -D warnings` is clean, unused imports included
- [ ] `cargo test` passes
- [ ] Commits follow [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/)
- [ ] Files are LF, not CRLF <!-- if you work across Windows and Linux, review with `git diff --ignore-cr-at-eol` -->
- [ ] Nothing used that is newer than the `rust-version` floor in `Cargo.toml`
- [ ] A new module is listed in CONTRIBUTING.md's architecture overview

<!-- Only if you added or changed a keybinding — the keymap is published in
     three places and a test enforces the first two: -->

- [ ] New keybinding added to `ui::help::SECTIONS`, to the key list in its test, and to the README table

## Notes for the Reviewer

<!-- Where to start reading, tradeoffs you weighed, alternatives you rejected,
     and anything deliberately left for a follow-up. Flagging a judgment call
     here is much cheaper than having it discovered in review. -->
