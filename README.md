# monitor

A modern `watch` command, inspired by [viddy](https://github.com/sachaos/viddy).
It runs a command over and over and shows its output full-screen, and adds:

- **Time machine**: every result is kept, so you can pause the view and go back to earlier results.
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
| `--max-history <N>` | how many results to keep (default 1000) |

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
| `d` `b` `w` `t` `H` | toggle diff / bell / wrap / title / history panel |

The history panel lists results newest first. `●` marks results whose output
changed, and a red id means the command exited with a non-zero status.
