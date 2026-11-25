mod app;
mod cli;
mod escapes;
mod input;
mod matcher;
mod runner;
mod signals;
mod terminal;
mod ui;

use std::io::{self, Write};
use std::time::Duration;

use clap::Parser;
use ratatui::backend::Backend;
use ratatui::crossterm::event::{self, Event};

use app::{App, ExitReason};
use cli::Cli;
use runner::Runner;
use signals::Signals;

fn stdout() -> Box<dyn Write> {
    Box::new(io::stdout())
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let (runner, rx) = Runner::start(&cli);
    let mut app = App::new(&cli, runner);
    let hostname = hostname();

    // A panic anywhere restores the terminal and stops the running commands.
    terminal::on_panic(app.runner.killer());
    let guard = match terminal::enter(stdout, !cli.no_mouse) {
        Ok(guard) => guard,
        Err(e) => {
            app.runner.kill_all();
            anyhow::bail!("failed to initialize terminal (is stdout a TTY?): {e}");
        }
    };
    // External termination signals end through the cleanup below, too.
    let signals = Signals::catch(app.runner.killer())?;
    let mut terminal = terminal::tui(io::stdout())?;
    let result = (|| -> anyhow::Result<Option<i32>> {
        loop {
            if let Some(sig) = signals.received() {
                return Ok(Some(sig));
            }
            while let Ok(msg) = rx.try_recv() {
                app.on_msg(msg);
            }
            if app.exit.is_some() {
                return Ok(None);
            }
            if std::mem::take(&mut app.redraw) {
                refresh_screen(&mut terminal)?;
            }
            terminal.draw(|f| ui::draw(f, &mut app, &hostname))?;

            if event::poll(Duration::from_millis(100))? {
                loop {
                    match event::read()? {
                        Event::Key(key) => app.on_key(key),
                        Event::Mouse(m) => app.on_mouse(m),
                        _ => {}
                    }
                    if app.exit.is_some() || !event::poll(Duration::ZERO)? {
                        break;
                    }
                }
            }
        }
    })();
    app.runner.kill_all();
    // Restore the terminal before printing anything (or exiting with -e).
    drop(terminal);
    drop(guard);
    if let Some(sig) = result? {
        std::process::exit(signals::exit_status(sig));
    }

    match app.exit {
        Some(ExitReason::Error(code)) => {
            print_last(&app);
            eprintln!("monitor: `{}` exited with status {code}", app.command);
            std::process::exit(1);
        }
        Some(ExitReason::Changed) => {
            print_last(&app);
        }
        _ => {}
    }
    Ok(())
}

fn print_last(app: &App) {
    if let Some(s) = app.history.last() {
        print!("{}", &*s.output);
    }
}

fn hostname() -> String {
    let mut buf = [0u8; 256];
    let ok = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0;
    if !ok {
        return String::new();
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

/// Redraws the whole screen (r: refresh), the same in track, monitor and git-graph.
/// Unlike `Terminal::clear`, this doesn't ask the terminal for the cursor position
/// (`\e[6n`), so a terminal that doesn't answer can't make it fail.
fn refresh_screen(terminal: &mut terminal::Tui<io::Stdout>) -> std::io::Result<()> {
    // Through the backend, so it also forgets what the screen showed
    Backend::clear(terminal.backend_mut())?;
    // An empty frame first, so the next frame repaints every cell.
    terminal.draw(|_| {})?;
    Ok(())
}
