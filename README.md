# monitor

A modern `watch` command, inspired by [viddy](https://github.com/sachaos/viddy).
It runs a command over and over and shows its output full-screen, and adds:

- **Time machine**: results are kept, so you can pause the view and go back to earlier results.
  A new result is recorded only when the output or exit status changes.
- **Diff highlighting**: shows which characters changed since the previous run (`-d` / `d`).
  The header lists the toggles that are on (`diff · bell · wrap`).
- **ANSI colors**: keeps the colors from the command's output.
- **Scrolling and search**: vim-style keys, and `/` regex search (smart case) in a small input box that jumps to matches while you type; `n`/`N` step through them and `Esc` hides the highlights. See [Shared search](#shared-search).
- **Suspend / refresh**: `s` pauses running the command, `r` refreshes (runs it right away).
- **Precise mode** (`-p`): starts a run every interval, even if the previous one is still going (at most 16 runs at once; further ticks are skipped while 16 are running).
- **watch-style exits**: `-e` exits on a non-zero status, `-g` exits when the output changes.
- **Safe with any command**: output is kept up to 16 MiB per run (a command that writes more, like
  `yes` or a `ping` without `-c`, is stopped there and the output says so), and commands run without
  the terminal, so a password prompt fails at once instead of hanging, and `--shell-options -i` works.
  Like track, only colors are kept from escape sequences, and a line redrawn with `\r` shows its
  latest text.

Unlike `watch`, `-w` turns wrapping *on*: monitor starts unwrapped, like track.

## Install

```sh
cargo install --path .
```

## Usage

```sh
monitor [OPTIONS] <COMMAND>...

monitor -n 1 -d 'kubectl get pods'
monitor -n 500ms ls -l --color=always
monitor -p -n 5s 'curl -s localhost:8080/health'
monitor -x -- git status --short       # run directly, no shell
```

| Option | Description |
| --- | --- |
| `-n, --interval <DUR>` | interval: `2`, `0.5`, `500ms`, `1m30s` (default `2s`) |
| `-d, --differences` | highlight changes |
| `-b, --bell` | ring the bell when the output changes |
| `-p, --precise` | run on a fixed schedule, even if the last run hasn't finished |
| `-t, --no-title` | hide the title bar |
| `-w, --wrap` | start with line wrapping on (off by default, like track; `w` toggles it) |
| `--no-mouse` | don't capture the mouse: drag uses the terminal's own selection, and the wheel scrolls only where the terminal supports alternate scroll mode |
| `-x, --exec` | run the command directly instead of through a shell |
| `-s, --shell <SHELL>` | shell to use (default `sh`, env `MONITOR_SHELL`) |
| `--shell-options <OPTIONS>` | extra shell options before `-c`, e.g. `"-i"` |
| `-e, --errexit` | exit when the command fails |
| `-g, --chgexit` | exit when the output changes |
| `-k, --keep-unchanged` | record every run, not only runs whose output or exit status changed |
| `--max-history <N>` | how many results to keep (default 1000) |
| `--max-history-size <SIZE>` | memory limit for kept results, e.g. `64M`, `1G` (default `128M`) |

Many programs turn off colors when their output isn't a terminal, so pass
`--color=always` (or the equivalent) to keep them.

## Keys

Press `?` in the app for this list (`j`/`k` or the wheel scroll it when it
doesn't fit; any other key closes it). The keys and their wording are the
same in track, monitor and git-graph (see the shared key guideline below).

**Navigation**

| Key | Action |
|---|---|
| `j`/`k`, `↓`/`↑` | Down / up |
| `ctrl-d`/`ctrl-u` | Half page down / up |
| `PgDn`/`PgUp`, `ctrl-f`/`ctrl-b` | Page down / up |
| `g`/`G` | Top / bottom |
| `h`/`l`, `←`/`→` | Left / right 4 columns (when not wrapping) |
| `H`/`L` | Left / right half a page (when not wrapping) |
| `home`/`end` | Line start / end (when not wrapping) |

**Search**

| Key | Action |
|---|---|
| `/` | Search (regex, smart case) |
| `n`/`N` | Next / previous match |
| `esc` | Hide search highlights, then back to the live view |

**Time machine**

| Key | Action |
|---|---|
| `space` | Toggle time machine |
| `J`/`K` | Older / newer result |
| `<`/`>` | Oldest / newest result |
| `esc` | Back to the live view |

**Input box** (the `/` box)

| Key | Action |
|---|---|
| `←`/`→`, `home`/`end`, `ctrl-a`/`ctrl-e` | Move the cursor |
| `ctrl-u` / `ctrl-w` | Delete to start / previous word |
| `↑`/`↓` | History |
| `enter` | Submit |
| `esc`, `ctrl-c` | Cancel (`backspace` on an empty box too) |

**Mouse**

| Key | Action |
|---|---|
| wheel | Scroll (`ctrl`: faster; over the history: older / newer result) |
| click | Select a result in the history |
| drag | Select text in the output or the history; copied on release (OSC 52) |
| shift + drag | The terminal's own selection |

**General**

| Key | Action |
|---|---|
| `s` | Suspend / resume running the command |
| `d` | Toggle diff |
| `b` | Toggle bell |
| `w` | Toggle wrap |
| `t` | Toggle title |
| `r` | Refresh (run the command now) |
| `?` | Help |
| `q`, `ctrl-c` | Quit |

While in time-machine mode, a history panel lists results newest first, by the
time each one started. `×N` shows how many unchanged runs a result covers, and a
red time means the command exited with a non-zero status. The bottom of the
panel shows how much memory the kept results use.

The output title says where the shown result differs from the one before it,
e.g. `changed: line 28` or `changed: 3 lines from line 12`. A new result is
only kept when the output (or exit status) changed, but the change can be
scrolled out of view, such as a `Restarting (1) 44 seconds ago` counter in
`docker ps -a`; the title tells you where to look. Turn on `d` to highlight
the changed characters.

## Shared key guideline

`track`, `monitor` and `git-graph` follow one key guideline, so the same
keys do the same thing in all three:

| Key | Meaning |
|---|---|
| `j`/`k`, `↓`/`↑` | Down / up |
| `ctrl-d`/`ctrl-u` | Half page down / up |
| `PgDn`/`PgUp`, `ctrl-f`/`ctrl-b` | Page down / up |
| `g`/`G` | Top / bottom |
| `h`/`l` | Left / right 4 columns |
| `H`/`L` | Left / right half a page |
| `home`/`end` | Line start / end (not top / bottom) |
| `/`, `n`/`N` | Search (regex, smart case), next / previous match |
| `esc` | Hide search highlights, then go back |
| `w` | Toggle wrap (track, monitor) |
| `r` | Refresh: redraw the screen, and run the command again (monitor) or reload the repository (git-graph) |
| `?` | Help, in the same window everywhere (`j`/`k` or the wheel scroll it; any other key closes it) |
| `q`, `ctrl-c` | Quit (in an input box, `ctrl-c` cancels it like `esc`) |
| mouse wheel | Scroll (`ctrl` + wheel: faster) |
| mouse drag | Select text and copy it with OSC 52 on release (up to 512 KB) |
| shift + drag | The terminal's own selection |

Other keys are specific to one tool, and no key just repeats another one.
In `git-graph`'s commit list, which doesn't scroll sideways, `H` jumps to
HEAD.

## Shared search

`/` opens the same small input box near the top of the screen in all three
(in the style of snacks.nvim's input):

```
          ╭───────────────────── Search ─────────────────────╮
          │  error█                               12 matches │
          ╰──── enter search · ↑/↓ history · esc cancel ─────╯
```

- Patterns are regular expressions with smart case (case-insensitive unless
  the pattern has an uppercase letter; the letter of an escape such as `\W`
  or `\S` doesn't count); an invalid regex is matched literally.
- While you type, matches are highlighted, the view moves to the first match
  from where you started, and the box shows how many there are.
- `enter` keeps the search; `esc` cancels it and goes back to where you were;
  an empty search clears it.
- Editing: `←`/`→`, `home`/`end`, `ctrl-a`/`ctrl-e` move the cursor,
  `ctrl-u`/`ctrl-w` delete to the start / the previous word, `↑`/`↓` browse
  earlier searches, and `backspace` on an empty box closes it.
- `n`/`N` move to the next / previous match. The current match's line is
  highlighted (in `git-graph` it is the selected commit), and the view only
  scrolls when the match is off screen, keeping 3 lines of context. Scroll
  the current match out of view and `n`/`N` start from the screen instead.
- The main panel's title shows the search and the current match, e.g.
  `/error  3/12`.
- Matches are highlighted in black on yellow. `esc` hides the highlights but
  keeps the search (the title then says `hidden`), like vim's `:noh`; `n`/`N`
  show them again and jump to the next / previous match.

## Shared screen layout

All three draw the same frame:

```
 tool  what it shows  state · flags that are on              tool-specific info
╭ Panel  details ─────────────────────────────────────── /search  current/total ╮
│ content                                                                       █
│                                                                               ║
╰──────────────────────────────────────────────────────────────────────────────╯
 j/k down/up  g/G top/bottom  / search  n/N next/prev  tool keys…  esc back  ? help  q quit
```

- The header starts with the tool's name on a cyan badge, then what it shows
  (cut short with `…` when space runs out), its state, and the flags that
  are on (e.g. `wrap`, `diff · bell · wrap`). The tool-specific info on the
  right appears when everything fits.
- Panels have rounded borders; the focused one is cyan.
- A panel whose content doesn't fit has a scrollbar on its right border, in
  the border's color (cyan when the panel is focused). Nothing is drawn over
  the bottom border.
- The footer lists keys in the same order everywhere: movement, search, the
  tool's most used keys (`?` lists them all), `esc` back where it applies,
  then `? help` and `q quit`.
  Keys are cyan and their descriptions gray; on a narrow terminal the tool's
  own keys are dropped first, then `? help`, and a hint is never cut in
  half. A status message (yellow) replaces the hints until the next key.
- The mouse is captured, so the wheel scrolls and a drag selects text.
  `--no-mouse` leaves it to the terminal and asks for alternate scroll mode,
  so the wheel still scrolls where the terminal supports it.
- However the tool ends (`q`, `ctrl-c`, a termination signal from outside
  such as `kill` or closing the terminal or tmux pane, or a crash), the
  terminal is restored: normal screen, cursor, keyboard and mouse.
- Double-width characters (Chinese, Japanese, Korean) stay whole, including
  under the edge of a popup and in terminals that don't blank the other half
  of a half-overwritten one, such as Neovim's terminal (toggleterm.nvim).

## Memory

Results are kept for the time machine up to two limits: `--max-history`
results and `--max-history-size` bytes. When either is exceeded, the oldest
results are dropped first.

By default, a run whose output and exit status match the latest result is not
stored. It is merged into that result instead, and the title shows how many
runs it covers and when it was last seen (`×12 until 12:05:00`). Commands
whose output rarely changes therefore use almost no memory, even over very
long runs. With `--keep-unchanged`, every run is recorded, but unchanged
results still share memory with the previous one.

## Copying text

Drag over the output to select text. The view stays frozen while you drag,
and dragging past the top or bottom edge scrolls. When you release the
button, the text is copied to the system clipboard with OSC 52, and wrapped
lines are joined back into one line. Windows Terminal, iTerm2, kitty,
WezTerm, Alacritty and foot support OSC 52. Inside tmux, run
`tmux set -s set-clipboard on`.

In time machine, you can also drag over the history list to copy result
times; a plain click there picks a result.

Shift + drag (Option + drag in iTerm2) still gives your terminal's own
selection.

## Development

```sh
cargo fmt
cargo clippy --all-targets
cargo test     # layout, diff, search, change summary and editor tests
```

| File | Contents |
|---|---|
| `src/runner.rs` | Running the command on a schedule; snapshots of each run |
| `src/app.rs` | Application state: history, time machine, scrolling, search, selection |
| `src/ui.rs` | Rendering with ratatui |
| `src/cli.rs` | Command-line options |
| `src/escapes.rs` | Removes escape sequences other than colors before parsing (shared with track) |
| `src/terminal.rs` | Entering and restoring the terminal, panics, OSC 52 copying (shared) |
| `src/signals.rs` | Quitting cleanly on termination signals (shared) |
| `src/input.rs` | Search box line editor and history (shared) |
| `src/matcher.rs` | Smart-case regex search patterns (shared) |
| `src/main.rs` | Terminal setup and the event loop |

### Code style

track, monitor and git-graph share one code style, so a change can be
carried between them:

- `rustfmt.toml` is the same in all three; run `cargo fmt` before committing.
- `cargo clippy --all-targets` must report nothing, and `cargo test` must pass.
- `src/input.rs` (line editor and history) and `src/matcher.rs` (smart-case
  regex search) are identical copies; change them in all three together.
- The same `[profile.release]` (LTO, one codegen unit, stripped) and
  `rust-version = "1.88"`; dependencies are listed alphabetically, with the
  same versions where they overlap.
