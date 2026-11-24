# monitor

A modern `watch` command, inspired by [viddy](https://github.com/sachaos/viddy).
It runs a command over and over and shows its output full-screen, and adds:

- **Time machine**: results are kept, so you can pause the view and go back to earlier results.
  A new result is recorded only when the output or exit status changes.
- **Diff highlighting**: shows which characters changed since the previous run (`-d` / `d`).
- **ANSI colors**: keeps the colors from the command's output.
- **Scrolling and search**: vim-style keys, and `/` regex search (smart case) in a small input box that jumps to matches while you type; `n`/`N` step through them and `Esc` hides the highlights. See [Shared search](#shared-search).
- **Suspend / run now**: `s` pauses running the command, `r` runs it right away.
- **Precise mode** (`-p`): starts a run every interval, even if the previous one is still going.
- **watch-style exits**: `-e` exits on a non-zero status, `-g` exits when the output changes.

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
| `-w, --unfold` | don't wrap lines (scroll horizontally instead) |
| `--no-mouse` | don't capture the mouse: plain drag uses the terminal's own selection, but no wheel scrolling or clicking |
| `-x, --exec` | run the command directly instead of through a shell |
| `-s, --shell <SHELL>` | shell to use (default `sh`, env `MONITOR_SHELL`) |
| `--shell-options <OPTS>` | extra shell options before `-c`, e.g. `"-i"` |
| `-e, --errexit` | exit when the command fails |
| `-g, --chgexit` | exit when the output changes |
| `-k, --keep-unchanged` | record every run, not only runs whose output or exit status changed |
| `--max-history <N>` | how many results to keep (default 1000) |
| `--max-history-size <SIZE>` | memory limit for kept results, e.g. `64M`, `1G` (default `128M`) |

Many programs turn off colors when their output isn't a terminal, so pass
`--color=always` (or the equivalent) to keep them.

## Keys

| Key | Action |
| --- | --- |
| `q`, `Ctrl-C` | quit |
| `?` | help |
| `space` | toggle time machine |
| `J` / `K`, `Shift-↓` / `Shift-↑` | older / newer result |
| `<` / `>` | oldest / newest result |
| `Esc` | hide the search highlights, or go back to the live view |
| `s` / `r` | suspend or resume / run now |
| `j` `k` `↑` `↓`, `Ctrl-D/U`, `PgDn/PgUp`, `Ctrl-F/B` | scroll a line / half a page / a page |
| `g` / `G` | top / bottom |
| mouse wheel | scroll (hold `Ctrl` to scroll faster); over the history panel, older / newer result |
| mouse click | select a result in the history panel |
| mouse drag | select text in the output or the history; it's copied to the clipboard when you release |
| shift + drag | your terminal's own text selection |
| `h` `l` `←` `→` | scroll sideways 4 columns (when lines aren't wrapped) |
| `H` / `L` | scroll sideways half a page |
| `Home` / `End` | line start / end (when lines aren't wrapped) |
| `/`, `n` / `N` | search (regex, smart case), next / previous match (`n` / `N` also show hidden highlights) |
| `d` `b` `w` `t` | toggle diff / bell / wrap / title |

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
| `j`/`k`, `↓`/`↑` | One line down / up |
| `ctrl-d`/`ctrl-u` | Half a page down / up |
| `PgDn`/`PgUp`, `ctrl-f`/`ctrl-b` | A full page down / up |
| `g` / `G` | Top / bottom |
| `h`/`l` | Sideways 4 columns |
| `H`/`L` | Sideways half a page |
| `home` / `end` | Line start / end (not top / bottom) |
| `/`, `n` / `N` | Search, next / previous match |
| `esc` | Hide the search highlights, then go back |
| `?` | Help, in the same window everywhere (`j`/`k` or the wheel scroll it; any other key closes it) |
| `q`, `ctrl-c` | Quit |
| mouse wheel | Scroll (`ctrl` + wheel: faster) |
| mouse drag | Select text and copy it with OSC 52 on release |
| shift + drag | The terminal's own selection |

Other keys are specific to one tool. Where a view doesn't scroll sideways,
`h`/`l`/`H`/`L` may do something else there (in `git-graph`'s commit list
they move between panes and jump to HEAD).

## Shared search

`/` opens the same small input box near the top of the screen in all three
(in the style of snacks.nvim's input):

```
          ╭───────────────────── Search ─────────────────────╮
          │  error█                               12 matches │
          ╰──── enter search · ↑/↓ history · esc cancel ─────╯
```

- Patterns are regular expressions with smart case (case-insensitive unless
  the pattern has an uppercase letter); an invalid regex is matched literally.
- While you type, matches are highlighted, the view jumps to the first match
  from where you started, and the box shows how many there are.
- `enter` keeps the search; `esc` cancels it and goes back to where you were;
  an empty search clears it.
- Editing: `←`/`→`, `home`/`end`, `ctrl-a`/`ctrl-e` move the cursor,
  `ctrl-u`/`ctrl-w` delete to the start / the previous word, `↑`/`↓` browse
  earlier searches, and `backspace` on an empty box closes it.
- The main panel's title shows the search and the current match, e.g.
  `/error  3/12`.
- Matches are highlighted in black on yellow. `esc` hides the highlights but
  keeps the search (the title then says `hidden`), like vim's `:noh`; `n` / `N`
  show them again and jump to the next / previous match.

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
