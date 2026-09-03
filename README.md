# gpick

An interactive terminal UI for cherry-picking commits between git branches — built to replace a couple of manual aliases (`gcp` / `gcars`) with something that shows you what you're about to pick, handles conflicts without leaving the tool, and re-authors + signs off every commit automatically.

## Why

The usual workflow — find a commit on another branch, `git cherry-pick` it, fix the author/date/sign-off, repeat — is tedious and easy to get wrong across many commits. gpick turns that into: pick a branch, select the commits you want with a live diff preview, hit enter, and let it work through the queue, pausing only when it actually needs you.

## Features

- **Branch list** with fuzzy search (`/`), multi-select (`Space`, `Shift+↑/↓` for ranges), and bulk delete (local or remote, with an animated progress footer for big batches)
- **Automatically hides branches with nothing to cherry-pick** — no commits ahead of base, or every commit already applied elsewhere (via `git cherry`) — toggle with `a` to see everything anyway
- **Commit list** with a live diff preview panel, filtering out commits that would land as empty picks
- **Cherry-pick execution** that re-authors, signs off (`-s`), and preserves the original commit date on every applied commit, then chains straight back to the branch list so you can keep going
- Handles the awkward edge cases: real merge conflicts, a cherry-pick that resolves to an empty diff, a stale remote-tracking ref pointing at a branch someone already deleted
- A `p` shortcut to push the current base onto `origin/master`, with a commit-count preview before confirming
- Fetches and prunes remote-tracking refs on startup so you're not picking a commit a branch no longer has

## Requirements

- `git` on your `PATH`
- A terminal that supports standard ANSI escape sequences (most do); `Shift+↑/↓` for range-select needs a terminal that reports shift modifiers on arrow keys (most modern ones do)

## Build

```sh
cargo build --release
```

The binary ends up at `target/release/gpick`.

## Usage

Run it from inside the repository you want to work in:

```sh
gpick
```

By default it detects the base branch (`origin/HEAD`, falling back to `main` then `master`). Override it explicitly:

```sh
gpick --base develop
```

### Keybindings

**Branch list**

| Key | Action |
|---|---|
| `↑`/`↓` | Move |
| `Shift+↑`/`↓` | Extend a range selection |
| `/` | Search |
| `a` | Toggle showing branches with nothing to pick |
| `Space` | Select/deselect the hovered branch |
| `Enter` | Open the branch's commit list |
| `Del` | Delete the hovered branch, or all selected ones |
| `p` | Push base to `origin/master` (hidden when there's nothing to push) |
| `q` | Quit |

**Commit list**

| Key | Action |
|---|---|
| `↑`/`↓` | Move |
| `Space` | Toggle the hovered commit |
| `Enter` | Cherry-pick the selected commits |
| `q` / `Esc` | Back to the branch list |

**On a conflict**

| Key | Action |
|---|---|
| `c` | Continue (after resolving the conflict and `git add`-ing it outside gpick) |
| `a` | Abort this commit and return to the commit list |
| `q` | Quit |

## Development

```sh
cargo test      # unit + integration tests, uses real temp git repos
cargo clippy
```

## License

MIT — see [LICENSE](LICENSE).
