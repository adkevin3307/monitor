# monitor

A modern `watch` command, inspired by [viddy](https://github.com/sachaos/viddy).
It runs a command over and over and shows its output full-screen, and adds:

- **Time machine**: results are kept, so you can pause the view and go back to earlier results.
  A new result is recorded only when the output or exit status changes.
- **Diff highlighting**: shows which characters changed since the previous run (`-d` / `d`).
- **ANSI colors**: keeps the colors from the command's output.
- **Scrolling and search**: vim-style keys, smart-case `/` search, and `n`/`N` to jump between matches.
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
| `Esc` | clear search, or go back to the live view |
| `s` / `r` | suspend or resume / run now |
| `j` `k` `↑` `↓`, `Ctrl-D/U`, `PgDn/PgUp`, `gg` / `G` | scroll |
| `h` `l` `←` `→`, `0` | scroll horizontally (when lines aren't wrapped) |
| `/`, `n` / `N` | search, next / previous match |
| `d` `b` `w` `t` | toggle diff / bell / wrap / title |

While in time-machine mode, a history panel lists results newest first. `●` marks results whose output
changed, and a red id means the command exited with a non-zero status. The
bottom of the panel shows how much memory the kept results use.

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
