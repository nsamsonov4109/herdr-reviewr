<div align="center">
  <sup>Special thanks to:</sup>
  <br><br>
  <a href="https://getmoshi.app/?utm_source=herdr-reviewr&utm_medium=readme&utm_campaign=sponsor"><img alt="Moshi" width="120" src="assets/sponsors/moshi.svg"></a>

### [Moshi: Terminals weren’t made for phones. Moshi is.](https://getmoshi.app/?utm_source=herdr-reviewr&utm_medium=readme&utm_campaign=sponsor)
[A mobile terminal designed for AI coding agents.](https://getmoshi.app/?utm_source=herdr-reviewr&utm_medium=readme&utm_campaign=sponsor)<br>

  <hr />
</div>

# herdr-reviewr

> A fork of [persiyanov/herdr-reviewr](https://github.com/persiyanov/herdr-reviewr), released ahead of
> it with threads, folder review and suggested commits. See the [changelog](CHANGELOG.md).

[![CI](https://github.com/nsamsonov4109/herdr-reviewr/actions/workflows/ci.yml/badge.svg)](https://github.com/nsamsonov4109/herdr-reviewr/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/nsamsonov4109/herdr-reviewr)](https://github.com/nsamsonov4109/herdr-reviewr/releases/latest)
[![License](https://img.shields.io/github/license/persiyanov/herdr-reviewr)](LICENSE)

<p align="center">
  <a href="#install">install</a> · <a href="#quick-start">quick start</a> · <a href="#controls">controls</a> · <a href="#diff-scopes">scopes</a> · <a href="#configuration">configuration</a> · <a href="#limitations">limitations</a> · <a href="CHANGELOG.md">changelog</a>
</p>

A code-review pane for [herdr](https://herdr.dev). Read your agent's diff beside the chat,
comment on lines, and send the comments back.

![demo](assets/demo.gif)

- **Diff review** — the agent's changes, syntax-highlighted.
- **Unified and split views** — one column or side by side, flip with `|`.
- **Five diff scopes** — uncommitted, branch, since send, last turn, commits.
- **Last-turn diff** — only what the agent's latest turn changed.
- **Line comments** — on a line or a range, sent to the agent in one go.
- **Threads** — the agent answers each comment where it sits, and can comment on its own code.
- **Comments follow the code** — an edit moves them with their lines, or says what changed.
- **Viewed marks** — tick files off as you go; one the agent changes again comes back.
- **Text selection** — drag to copy, like an editor.
- **File viewer** — any file in the worktree.
- **Search** — files by name and code by content, powered by [fff](https://github.com/dmtrKovalenko/fff).
- **Find in file** — step through every match.
- **PR view** — the branch's pull request, read-only.
- **Markdown review** — `m` renders a `.md` file.
- **Themes** — 20, dark and light.

reviewr never edits your files, never sends on its own, and never posts to your PR.

## Requirements

- **herdr ≥ 0.9.3** and **git**.
- A terminal with **24-bit color**.
- **macOS, Linux, or Windows.**
- For the PR tab only: **`gh`**, **`glab`**, or **`az`** (with the `azure-devops` extension), logged in.

## Install

```bash
herdr plugin install nsamsonov4109/herdr-reviewr
```

reviewr opens on its own when herdr creates a workspace for a worktree. To open it yourself:

```bash
herdr plugin action invoke open --plugin persiyanov.reviewr
```

**To update**, reinstall. Your config stays:

```bash
herdr plugin uninstall persiyanov.reviewr && herdr plugin install nsamsonov4109/herdr-reviewr
```

**Without herdr**, run a [release binary](https://github.com/nsamsonov4109/herdr-reviewr/releases/latest)
on any repo. Everything but **Send** and the **last turn** scope works:

```bash
herdr-reviewr ~/some/repo
```

## Quick start

1. **Pick a file.** `j` / `k` in the file list, or `]` to jump from change to change.
2. **Focus the diff.** `Tab`.
3. **Select lines.** `v`, then `j` / `k`, or drag the line numbers.
4. **Comment.** `c`, type, `Enter`.
5. **Send.** `s` puts every comment in the input of the agent beside reviewr, the one in its tab.
   With none there and several in the workspace, a picker asks which.

The footer shows the next step. `?` lists every key.

To toggle reviewr with a key, add this to `~/.config/herdr/config.toml`:

```toml
[[keys.command]]
key = "cmd+r"   # many macOS terminals swallow alt+
type = "plugin_action"
command = "persiyanov.reviewr.toggle"
```

## Controls

Every key can be rebound ([Keybindings](#keybindings)).

**Getting around**

| Key | Action |
| --- | --- |
| `1` `2` `3` | Tab: Changes / All files / PR |
| `u` `b` `i` `t` `g` | Scope: uncommitted / branch / since send / last turn / commits |
| `B` | Pick the base branch |
| `G` | Pick commits |
| `j` `k` · `↑` `↓` | Move |
| `]` `[` | Next / previous change |
| `f` `F` | Next / previous file |
| `PageUp` `PageDown` | Page |
| `Ctrl+U` `Ctrl+D` | Half page |
| `Tab` | Switch pane |
| `→` `←` | Expand / collapse, or scroll sideways |
| `/` | Search files and code |
| `Ctrl+F` | Find in file |
| `:` | Go to line (`:0` first, `:$` last) |
| `w` | Wrap lines |
| `m` | Rendered / source markdown |
| `\|` | Unified / split diff (needs 137 columns) |
| `p` | Move the file list |
| `z` | Hide / show the file list |
| `<` `>` | Widen / narrow the file list |
| `r` | Refresh |
| `?` | All keys |
| `q` | Quit (asks if comments are unsent) |
| `Q` | Quit and drop unsent comments |

**Reviewing**

| Key | Action |
| --- | --- |
| `v` | Select lines |
| `c` | Comment |
| `e` | Edit the comment, or open the file in your editor |
| `d` | Delete comment |
| `a` | Reply in a sent comment's thread |
| `D` | Resolve a thread, or reopen it from the list |
| `A` | Go to the next new answer from the agent |
| `n` `N` | Next / previous comment, across files |
| `l` | List comments |
| `space` | Mark the file viewed, then open the next |
| `s` | Send new comments and replies to the agent |
| `y` | Copy comments |
| `esc` | Clear selection |

**Comment box**

| Key | Action |
| --- | --- |
| `Enter` | Save |
| `Esc` | Cancel |
| `Shift+Enter` · `Alt+Enter` · `Ctrl+J` | New line |

The usual editing keys work: arrows, `Home` / `End`, `Ctrl+A` / `Ctrl+E`, `Alt+b` / `Alt+f`,
`Ctrl+W` / `Ctrl+U` / `Ctrl+K`.

**PR tab**

| Key | Action |
| --- | --- |
| `j` `k` | Move through description and comments |
| `PageUp` `PageDown` | Scroll |
| `o` | Open in browser |
| `r` | Refresh |

**Mouse:** drag to copy, double-click a word, triple-click a line. Click or drag line numbers to
comment. Click files, tabs, and links. Scroll with the wheel.

## Threads

A sent comment stays on its line as a numbered thread. The send tells the agent how to answer
each one, and its answer appears under the line it is about. The pane's first send to an agent
also explains how the review goes, that your comments are views rather than orders it must
follow, and every command it can run; later sends end with the short version. Each message opens under its author, `you` or the agent's name, with a bar in that
author's color down its lines:

```text
herdr-reviewr --reply 3 --to 8e9a3 "It runs once the policy attaches."
```

An answer you haven't read yet marks its thread `new answer`, and its file `●` where opening the
file shows the thread. `A` goes to the next one, across files, and `n` / `N` step through every
comment the same way. An answer no file in the view shows, like one on a commit the view doesn't
hold, opens the comments list (`l`) on its thread, which reads under the list's rows.

`a` replies in a thread, and the next `s` sends your reply with any new comments. A reply to a
thread the open file doesn't show opens over the body, under its thread. A sent comment isn't
edited: you reply instead. `D` resolves a thread, taking it out of the diff. It stays in
the comments list (`l`), greyed, where `D` reopens it. Threads live in the pane, like comments,
and end when it closes.

When the agent edits a file, its comments move with their lines. A comment whose lines were
rewritten or deleted stays where they were, and its title says `code changed`: on a sent comment,
usually the agent acting on it. A draft like that goes out marked so.

A comment shows in every view that reads its lines, not only the one it was made in. Made on a
commit, it shows on a later commit that rewrote those lines, on the removed side, and in
**uncommitted** or **All files** while the worktree still reads them. Each view follows the
lines from the text the comment was made on, so an amended or rebased commit keeps it on its own
line, and where that line was rewritten the card says `code changed`. A view that lost the
lines leaves it to the comments list. Shown from another version, its
title says so, `from 1a2b3c4` or `from the worktree`, and a send names the commit a comment was
made on. A comment on a removed line counts in the base of the scope it was made in: another
scope shows it where its own base reads the line, `from branch` say, and never moves it.

When the agent rewrites the branch, as a rebase or an amend does, a comment made on one of its
commits moves to what the rewrite made of that commit: the commit with the same subject, or,
where several share it, the one `git range-diff` pairs it with. There it follows its lines, or
sits where they were with `code changed`, and a send names the new commit and its line there. A
comment on a commit the rewrite dropped or renamed stays where it was, and so does one on a
commit another branch still holds: checking out a branch rewrites nothing.

The agent can comment too, on any line of the review, and each send tells it how:

```text
herdr-reviewr --comment src/app.rs:120-124 "This retry is new; it covers the flaky socket."
```

Its comment opens a thread marked `new comment`, which `A` goes to. You reply, resolve or delete
it, but don't edit it.

The agent can also point you at commits to review together: by sha, by a range as git reads it
(`a..b` is the commits after `a` up to `b`), or by a list, the run from its oldest commit to its
newest. The header names a run by its ends, `1a2b3c4 to 5d6e7f8`, both included:

```text
herdr-reviewr --suggest 1a2b3c4,5d6e7f8 "The retry step, in both repos."
```

Nothing moves: the footer offers `G suggested commits`, and `G` opens the commit picker with
them selected and the agent's reason as its title, so `enter` takes them. Move the highlight
first and the selection goes: `enter` takes the commit you moved to. A pick from the picker
the suggestion opened, whichever commit it takes, answers it. A review in steps arrives as one
suggestion per step, and they wait in the order sent: `G` opens the first, titled `(1 of 3)`,
and each pick moves on to the next. A suggestion the list no longer holds whole, as after a
rebase, gives way to the next when `G` opens, and the status says so. In the
agent's tab, these commands find the pane without `--to`, and `herdr-reviewr --help` lists all
three.

## Viewed marks

`space` marks the open file viewed and opens the next one you haven't viewed. The file list dims
a viewed file with `✓` and counts them. When the file changes after that, its mark turns to `!`
until you view it again. On a viewed file, `space` unmarks it; on a folder, it marks every file
inside. Marks live in the pane, like comments.

## The three tabs

- **Changes** — files the current scope changed.
- **All files** — every file in the worktree. You can comment here too. Git-ignored files are dimmed.
- **PR** — the branch's pull request (merge request on GitLab): state, checks, description, comments.

## Diff scopes

- **uncommitted** — changes since the last commit, including new files.
- **branch** — everything since the branch left its base, commits included. The base is the
  default branch until you pick another ([Base branch](#base-branch)).
- **since send** — what changed after your last send: your next review round. `i` again goes
  back to the scope you came from, so one key flips between the round and all the changes.
- **last turn** — changes since the agent's latest turn started.
- **commits** — one commit or a range, picked with `G`. In a folder of repositories, the
  picker lists every repo's commits over its base, newest first ([A folder of
  repositories](#a-folder-of-repositories)).

reviewr starts in **uncommitted** (`default_scope` changes that). Git-ignored files never show
in **Changes**.

## A folder of repositories

Start reviewr in a folder that isn't a repo but holds repos directly, like a workspace of
clones, and it reviews all of them at once. Each repo is a folder in the file list. Paths start
at the folder, so the agent working there can open any file a comment names. Toggle opens
reviewr on such a folder too.

Each repo diffs against its own last commit and its own base. **last turn** covers them all: a
turn starts from a snapshot of every repo, and shows what it changed in any of them. The **PR**
tab needs one repo: open reviewr inside it for that.

`G` lists every repo's commits over its base, newest first, each with its repo. A change made in
several repos at once usually lands as one commit title in each, so `enter` opens the
highlighted commit together with the newest commit of that title in every other repo. `v`
selects that step instead, and then adds or drops the highlighted row; `esc` clears the
selection. Two commits picked in one repo open as the run between them. The picker opens on
what is picked now, or on a suggestion: that selection lasts until you move the highlight.

## Configuration

Flags on the pane command:

| Flag | Default | Meaning |
| --- | --- | --- |
| `--base <ref>` | auto | base for the branch scope, overrides the pick |
| `--theme <name>` | `catppuccin` | color theme |
| `--wrap <on\|off>` | `on` | wrap long lines |

Everything else goes in `~/.config/herdr/plugins/config/persiyanov.reviewr/config.toml`. Saving
it applies at once. An invalid file blocks the pane with the error until you fix it.

```toml
theme = "tokyo-night"
default_scope = "branch"            # uncommitted | branch | last-turn
default_base = ["develop", "main"]  # the first one each repo has
markdown_view = "rendered"          # source | rendered
diff_view = "split"                 # unified | split
navigator_position = "bottom"       # right | bottom | left | top
toggle_placement = "overlay"        # split | overlay | zoomed | tab
toggle_direction = "down"           # right | down, for split
auto_open = false
editor = "code -g {file}:{line}"
url_opener = "browser-bridge --new-tab {url}"
review_prefix = "Fix each comment, then run the tests."
github_host = "github.example.com"
gitlab_host = "git.corp.example"
azure_devops_host = "tfs.corp.example"

[keybindings]
comment = ["c", "ㅊ"]
```

### Theme

Dark: `catppuccin`, `catppuccin-frappe`, `catppuccin-macchiato`, `dracula`, `nord`, `gruvbox`,
`one-dark`, `solarized`, `monokai`, `tokyo-night`, `rose-pine`, `ayu`, `everforest`.

Light: `catppuccin-latte`, `gruvbox-light`, `one-light`, `solarized-light`, `github-light`,
`tokyo-night-day`, `rose-pine-dawn`.

Pick one that matches your terminal's background. Shared names match herdr's colors.

### Markdown and diff view

`m` and `|` stick for every file you open until you press them again. `markdown_view` and
`diff_view` set how a new pane starts.

Split needs a diff pane 137 columns wide: hide the file list (`z`), move it below (`p`), or open
reviewr zoomed or as an overlay. A narrower pane shows unified until it widens.

### File list

`navigator_position` or `p` moves it. `<` / `>` or dragging its edge resizes it. `z` hides it.

### Base branch

The branch scope compares against the repo's default branch. To pick another, press `B` or click
the base name in the header. Type to filter, or type any commit reference like `HEAD~2`. The
pick holds for this worktree until you change it. `--base` overrides it.

`default_base` names branches to try before the default branch. Each repo uses the first one it
has, so one line covers repos with different trunks. A pick still wins over it.

In a folder of repositories, a pick goes to every repo that has that branch. The header names
the base most changed repos use, and counts any that use another.

### Editor

`e` opens the file at your line in `$VISUAL`, `$EDITOR`, or git's `core.editor`. Common editors
open at the right line. A terminal editor takes the pane until you quit. For anything else, set
`editor`; `{file}` and `{line}` are filled in.

### URL opener

Links open with `open`, `xdg-open`, or the Windows default browser. Under `herdr --remote`, set
`url_opener` to something that reaches your browser. The URL is passed as one argument, never
through a shell, so don't use `ssh host open {url}`.

### Review prefix

`review_prefix` goes above your comments on every send and copy. It arrives as pasted text, so
Claude Code won't run a `/command` in it. To run one, type it in the agent's input before you
press `s`.

### Keybindings

Each action takes a list of keys that replaces its defaults. The footer shows the first. Extra
keys help non-Latin layouts: bind the character your layout types on the same key.

| Action | Default |
| --- | --- |
| `down` / `up` | `j` / `k` |
| `next-hunk` / `prev-hunk` | `]` / `[` |
| `next-file` / `prev-file` | `f` / `F` |
| `scope-uncommitted` / `scope-branch` / `scope-last-turn` / `scope-commits` | `u` / `b` / `t` / `g` |
| `scope-since-send` | `i` |
| `base-pick` / `commit-pick` | `B` / `G` |
| `tab-changes` / `tab-all-files` / `tab-pr` | `1` / `2` / `3` |
| `wrap` | `w` |
| `rendered` | `m` |
| `split` | `\|` |
| `navigator-position` | `p` |
| `navigator-hide` | `z` |
| `navigator-grow` / `navigator-shrink` | `<` / `>` |
| `select` | `v` |
| `comment` | `c` |
| `edit` / `delete` | `e` / `d` |
| `reply` / `resolve` / `next-answer` | `a` / `D` / `A` |
| `next-comment` / `prev-comment` | `n` / `N` |
| `comments` | `l` |
| `viewed` | `space` |
| `search` | `/` |
| `find` | `ctrl+f` |
| `goto-line` | `:` |
| `keys` | `?` |
| `send` | `s`, `S` |
| `copy` | `y`, `Y` |
| `open-pr` | `o` |
| `refresh` | `r` |
| `quit` | `q` |
| `quit-discard` | `Q` |

A key is one character, a named key like `space` or `pagedown`, or a `ctrl+` / `alt+` chord.
`Tab`, `Esc`, and `Enter` are fixed.

### GitHub, GitLab, and Azure DevOps

The PR tab reads the `upstream` remote, else `origin`, so forks work. Self-hosted servers need
`github_host`, `gitlab_host`, or `azure_devops_host`, matching the remote's hostname exactly.
SSH aliases like `github.com-work` don't match.

### Pane placement

The toggle opens reviewr right of your agent. `toggle_placement` changes that: `overlay` covers
the tab, `zoomed` fills it, `tab` opens a new one.

reviewr opens on its own in a new worktree's workspace, with `split` or `tab` only, and leaves
your agent focused. `auto_open = false` turns that off.

### Layouts

To place reviewr in a herdr layout, give a pane `command = "herdr-reviewr"`. If `~/.local/bin`
isn't on your `PATH`, use `~/.local/state/herdr/plugins/persiyanov.reviewr/bin/herdr-reviewr`.
On Windows, point it at the plugin's own `bin` folder.

## Limitations

- **Colors need a 24-bit terminal**, and the theme must be matched to the background by hand.
- **Added and removed lines differ only by color.**
- **Copying** goes through the terminal (OSC 52), the way herdr's own copy does, so it reaches
  your machine over SSH. A terminal that ignores OSC 52 needs `pbcopy`, `wl-copy`, `xclip`, or
  `xsel` where reviewr runs (Windows needs none). Send works without.
- **Windows:** a reviewr started by hand in a PowerShell pane isn't recognized, so toggle opens
  another. Toggle may open on the pane's launch folder, not where you `cd`'d.
- **Network drives send no file events.** macOS and Linux check every five seconds. On Windows,
  press `r`.
- **Send needs an agent in the workspace**, and refuses one at a permission prompt. Your
  comments stay.
- **last turn shows all of a turn or nothing.** If the agent writes the instant a turn starts,
  the scope shows nothing for that turn and says why.
- **The PR tab** supports GitHub, GitLab, and Azure DevOps only. A merged or closed PR shows as
  history. Each comment list shows its newest 100.
- **A background pane** catches up 30 seconds after a change, or at once when shown.
- **A rename can pair with the wrong file** until you press `r`.
- **Comments live only in the pane.** Closing it loses unsent ones, every thread, and the
  viewed marks. Send delivers all or none.
- **An agent answers and comments on macOS and Linux only.** On Windows, sends carry no answer
  line.
- **A comment follows its lines within their file.** Moved to another file, or too short or too
  common to tell apart, they leave it where they were, marked `code changed`.
- **Files over 2 MB or 50,000 lines** aren't diffed. Neither are binary files.
- **A folder of repositories** reviews only the repos directly inside it, and no file beside
  them. Its PR tab doesn't work yet.

## Building from source

See [CONTRIBUTING.md](CONTRIBUTING.md). To run your build in herdr:

```bash
git clone https://github.com/nsamsonov4109/herdr-reviewr
cd herdr-reviewr
just install   # build to bin/herdr-reviewr, re-signed on macOS
herdr plugin uninstall persiyanov.reviewr
herdr plugin link .
```

Toggle the pane off and on after every `just install`.

## Roadmap

JSON export, more named keys, automatic light/dark theme, more themes (`kanagawa`,
`vesper`, a dark `github`), a theme that follows the terminal.

## Sponsors

reviewr is built by one person. If it saves you time,
[sponsor it](https://github.com/sponsors/persiyanov). Company sponsors get their logo here.

## License

[MIT](LICENSE). Syntax highlighting by [syntect](https://github.com/trishume/syntect) and
[two-face](https://github.com/CosmicHorrorDev/two-face), whose bundled themes color most syntax.

Bundled syntax themes in `assets/`, each under its own license:

- [Catppuccin Mocha](https://github.com/catppuccin/bat) — MIT.
- [Tokyo Night](https://github.com/folke/tokyonight.nvim) (`tokyo-night`, `tokyo-night-day`) — Apache-2.0.
- [Rosé Pine](https://github.com/rose-pine/tm-theme) (`rose-pine`, `rose-pine-dawn`) — MIT.
- [ayu](https://github.com/dempfi/ayu) (`ayu`) — MIT, converted from its Sublime color scheme.
- [Everforest](https://github.com/sainnhe/everforest) (`everforest`) — MIT, built from its palette and highlight groups.
