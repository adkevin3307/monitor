use std::collections::HashSet;
use std::io::Read;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use chrono::{DateTime, Local};

use crate::cli::Cli;

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub id: usize,
    pub start: DateTime<Local>,
    pub duration: Duration,
    pub output: String,
    /// Exit code (128 + signal when killed); `None` when it failed to start.
    pub code: Option<i32>,
    /// Whether the output differs from the previous snapshot.
    pub changed: bool,
}

impl Snapshot {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

pub enum Msg {
    Started(usize),
    Finished(Snapshot),
}

/// Handle used by the UI to control the background runner.
pub struct Runner {
    suspended: Arc<AtomicBool>,
    trigger: Sender<()>,
    children: Arc<Mutex<HashSet<i32>>>,
}

#[derive(Clone)]
struct Spec {
    program: String,
    args: Vec<String>,
}

impl Runner {
    pub fn start(cli: &Cli) -> (Runner, Receiver<Msg>) {
        let (msg_tx, msg_rx) = mpsc::channel();
        let (trigger_tx, trigger_rx) = mpsc::channel();
        let suspended = Arc::new(AtomicBool::new(false));
        let children = Arc::new(Mutex::new(HashSet::new()));

        let spec = if cli.exec {
            Spec { program: cli.command[0].clone(), args: cli.command[1..].to_vec() }
        } else {
            let mut args: Vec<String> =
                cli.shell_options.split_whitespace().map(str::to_owned).collect();
            args.push("-c".into());
            args.push(cli.command_display());
            Spec { program: cli.shell.clone(), args }
        };

        let interval = cli.interval;
        let precise = cli.precise;
        let susp = suspended.clone();
        let kids = children.clone();
        thread::spawn(move || {
            run_loop(spec, interval, precise, susp, kids, msg_tx, trigger_rx)
        });

        (Runner { suspended, trigger: trigger_tx, children }, msg_rx)
    }

    pub fn set_suspended(&self, value: bool) {
        self.suspended.store(value, Ordering::SeqCst);
        if !value {
            self.run_now();
        }
    }

    pub fn run_now(&self) {
        let _ = self.trigger.send(());
    }

    /// Kill every command that is still running.
    pub fn kill_all(&self) {
        let pids: Vec<i32> = self.children.lock().map(|c| c.iter().copied().collect()).unwrap_or_default();
        for pid in pids {
            // Each child is its own process group leader; kill the whole group.
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
    }
}

fn run_loop(
    spec: Spec,
    interval: Duration,
    precise: bool,
    suspended: Arc<AtomicBool>,
    children: Arc<Mutex<HashSet<i32>>>,
    tx: Sender<Msg>,
    trigger: Receiver<()>,
) {
    let mut id = 0usize;
    let mut forced = true;
    loop {
        let tick = Instant::now();
        if forced || !suspended.load(Ordering::SeqCst) {
            id += 1;
            if precise {
                let (spec, children, tx) = (spec.clone(), children.clone(), tx.clone());
                thread::spawn(move || run_once(id, &spec, &children, &tx));
            } else if !run_once(id, &spec, &children, &tx) {
                return;
            }
        }
        forced = false;

        let deadline = if precise { tick + interval } else { Instant::now() + interval };
        loop {
            match trigger.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(()) => {
                    forced = true;
                    break;
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }
}

/// Runs the command once. Returns false when the UI has gone away.
fn run_once(id: usize, spec: &Spec, children: &Mutex<HashSet<i32>>, tx: &Sender<Msg>) -> bool {
    if tx.send(Msg::Started(id)).is_err() {
        return false;
    }
    let start = Local::now();
    let clock = Instant::now();
    let (output, code) = match execute(spec, children) {
        Ok(r) => r,
        Err(e) => (format!("monitor: failed to run `{}`: {e}\n", spec.program), None),
    };
    tx.send(Msg::Finished(Snapshot {
        id,
        start,
        duration: clock.elapsed(),
        output,
        code,
        changed: false,
    }))
    .is_ok()
}

fn execute(spec: &Spec, children: &Mutex<HashSet<i32>>) -> std::io::Result<(String, Option<i32>)> {
    // stdout and stderr share one pipe so their output stays interleaved.
    let (mut reader, writer) = std::io::pipe()?;
    let mut cmd = Command::new(&spec.program);
    cmd.args(&spec.args)
        .stdin(Stdio::null())
        .stdout(writer.try_clone()?)
        .stderr(writer)
        .process_group(0);
    let mut child = cmd.spawn()?;
    // Drop our copies of the write end, otherwise reading never hits EOF.
    drop(cmd);

    let pid = child.id() as i32;
    children.lock().unwrap().insert(pid);

    let mut buf = Vec::new();
    let read = reader.read_to_end(&mut buf);
    let status = child.wait();
    children.lock().unwrap().remove(&pid);
    read?;
    let status = status?;

    let code = status.code().or_else(|| status.signal().map(|s| 128 + s));
    Ok((String::from_utf8_lossy(&buf).into_owned(), code))
}
