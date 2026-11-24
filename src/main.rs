mod app;
mod cli;
mod input;
mod matcher;
mod runner;
mod ui;

use std::time::Duration;

use clap::Parser;
use ratatui::crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event};
use ratatui::crossterm::execute;

use app::{App, ExitReason};
use cli::Cli;
use runner::Runner;

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let (runner, rx) = Runner::start(&cli);
    let mut app = App::new(&cli, runner);
    let hostname = hostname();

    let mut terminal = ratatui::try_init()
        .map_err(|e| anyhow::anyhow!("failed to initialize terminal (is stdout a TTY?): {e}"))?;
    let mouse = !cli.no_mouse;
    if mouse {
        execute!(std::io::stdout(), EnableMouseCapture)?;
    }
    let result = (|| -> anyhow::Result<()> {
        loop {
            while let Ok(msg) = rx.try_recv() {
                app.on_msg(msg);
            }
            if app.exit.is_some() {
                return Ok(());
            }
            if std::mem::take(&mut app.redraw) {
                terminal.clear()?;
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
    if mouse {
        let _ = execute!(std::io::stdout(), DisableMouseCapture);
    }
    ratatui::restore();
    app.runner.kill_all();
    result?;

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
