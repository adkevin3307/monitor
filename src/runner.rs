use std::collections::HashSet;
use std::io::Read;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use chrono::{DateTime, Local};

use crate::cli::Cli;

/// Output kept from one run, in bytes and in lines. A command that writes more (`yes`, a
/// `ping` without `-c`) is stopped there instead of filling memory; the output says it was
/// cut. (Laying out a result takes many times its size, so these stay moderate.)
const OUTPUT_LIMIT: usize = 1024 * 1024;
const LINE_LIMIT: usize = 50_000;
/// Runs at the same time in precise mode. While this many are still running, further
/// ticks are skipped, so a command that hangs can't pile up processes.
const MAX_IN_FLIGHT: usize = 16;

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub id: usize,
    pub start: DateTime<Local>,
    pub duration: Duration,
    /// Shared with the previous snapshot when the output is identical.
    pub output: Arc<str>,
    /// Exit code (128 + signal when killed); `None` when it failed to start.
    pub code: Option<i32>,
    /// Whether the output differs from the previous snapshot.
    pub changed: bool,
    /// Number of runs this snapshot stands for (unchanged runs are merged in).
    pub runs: usize,
    /// Start time of the most recent run merged into this snapshot.
    pub last_seen: DateTime<Local>,
    /// Id of the newest run merged into this snapshot. In precise mode a run that
    /// started earlier can finish later; it belongs before this snapshot, not after.
    pub last_id: usize,
}

impl Snapshot {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }

    /// Same output and exit status.
    pub fn same_result(&self, other: &Snapshot) -> bool {
        self.code == other.code && *self.output == *other.output
    }

    /// Folds an unchanged run (`other`) into this snapshot.
    pub fn absorb(&mut self, other: &Snapshot) {
        self.runs += other.runs;
        self.last_seen = self.last_seen.max(other.last_seen);
        self.last_id = self.last_id.max(other.last_id);
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
            Spec {
                program: cli.command[0].clone(),
                args: cli.command[1..].to_vec(),
            }
        } else {
            let mut args: Vec<String> = cli
                .shell_options
                .split_whitespace()
                .map(str::to_owned)
                .collect();
            args.push("-c".into());
            args.push(cli.command_display());
            Spec {
                program: cli.shell.clone(),
                args,
            }
        };

        let interval = cli.interval;
        let precise = cli.precise;
        let susp = suspended.clone();
        let kids = children.clone();
        thread::spawn(move || run_loop(spec, interval, precise, susp, kids, msg_tx, trigger_rx));

        (
            Runner {
                suspended,
                trigger: trigger_tx,
                children,
            },
            msg_rx,
        )
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
        kill_all(&self.children);
    }

    /// Something that kills every running command, for the panic hook.
    pub fn killer(&self) -> impl Fn() + Send + Sync + 'static {
        let children = self.children.clone();
        move || kill_all(&children)
    }
}

fn kill_all(children: &Mutex<HashSet<i32>>) {
    // try_lock: this also runs from the panic hook, maybe while the lock is held
    let pids: Vec<i32> = match children.try_lock() {
        Ok(c) => c.iter().copied().collect(),
        Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner().iter().copied().collect(),
        Err(std::sync::TryLockError::WouldBlock) => return,
    };
    for pid in pids {
        // Each child leads its own session and process group; kill the whole group.
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
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
    let in_flight = Arc::new(AtomicUsize::new(0));
    loop {
        let tick = Instant::now();
        if forced || !suspended.load(Ordering::SeqCst) {
            if !precise {
                id += 1;
                if !run_once(id, &spec, &children, &tx) {
                    return;
                }
            } else if in_flight.load(Ordering::SeqCst) < MAX_IN_FLIGHT {
                id += 1;
                in_flight.fetch_add(1, Ordering::SeqCst);
                let (spec, kids, done) = (spec.clone(), children.clone(), in_flight.clone());
                let run_tx = tx.clone();
                let started = thread::Builder::new().spawn(move || {
                    run_once(id, &spec, &kids, &run_tx);
                    done.fetch_sub(1, Ordering::SeqCst);
                });
                if let Err(e) = started {
                    // Out of threads: report a failed run instead of panicking
                    in_flight.fetch_sub(1, Ordering::SeqCst);
                    let now = Local::now();
                    let failed = Snapshot {
                        id,
                        start: now,
                        duration: Duration::ZERO,
                        output: format!("monitor: could not start a run: {e}\n").into(),
                        code: None,
                        changed: false,
                        runs: 1,
                        last_seen: now,
                        last_id: id,
                    };
                    if tx.send(Msg::Finished(failed)).is_err() {
                        return;
                    }
                }
            }
        }
        forced = false;

        let base = if precise { tick } else { Instant::now() };
        // Wait for the next tick, or for "run now". (A huge interval just waits.)
        let woke = match base.checked_add(interval) {
            Some(deadline) => {
                trigger.recv_timeout(deadline.saturating_duration_since(Instant::now()))
            }
            None => trigger.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match woke {
            Ok(()) => {
                forced = true;
                // Presses made while a run was going count once, not once each
                while trigger.try_recv().is_ok() {}
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
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
        Err(e) => (
            format!("monitor: failed to run `{}`: {e}\n", spec.program),
            None,
        ),
    };
    tx.send(Msg::Finished(Snapshot {
        id,
        start,
        duration: clock.elapsed(),
        output: output.into(),
        code,
        changed: false,
        runs: 1,
        last_seen: start,
        last_id: id,
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
        .stderr(writer);
    // A session of its own, with no controlling terminal: something that reads the
    // terminal (a password prompt, an interactive shell's startup) fails at once instead
    // of being stopped for good in the background, and nothing can write over the UI.
    // The child still leads its process group, so `kill(-pid)` stops all of it.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = cmd.spawn()?;
    // Drop our copies of the write end, otherwise reading never hits EOF.
    drop(cmd);

    let pid = child.id() as i32;
    children.lock().unwrap().insert(pid);

    let (buf, cut, read) = read_limited(&mut reader);
    if cut || read.is_err() {
        // Don't wait for a command that is still writing into a pipe nobody reads
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
    drop(reader);
    let status = child.wait();
    children.lock().unwrap().remove(&pid);
    read?;
    let status = status?;

    let code = status.code().or_else(|| status.signal().map(|s| 128 + s));
    let mut output = String::from_utf8_lossy(&buf).into_owned();
    if cut {
        if !output.ends_with('\n') {
            output.push('\n');
        }
        output.push_str(&format!(
            "monitor: output cut at {} MiB or {LINE_LIMIT} lines; the command was stopped\n",
            OUTPUT_LIMIT >> 20
        ));
    }
    Ok((output, code))
}

/// Reads the command's output up to [`OUTPUT_LIMIT`] bytes or [`LINE_LIMIT`] lines.
/// Returns what was kept, whether it was cut short, and how reading ended.
fn read_limited(reader: &mut impl Read) -> (Vec<u8>, bool, std::io::Result<()>) {
    let mut buf = Vec::new();
    let mut chunk = vec![0; 64 * 1024];
    let mut lines = 0;
    loop {
        let n = match reader.read(&mut chunk) {
            Ok(0) => return (buf, false, Ok(())),
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return (buf, false, Err(e)),
        };
        let mut keep = n.min(OUTPUT_LIMIT - buf.len());
        for (i, _) in chunk[..keep]
            .iter()
            .enumerate()
            .filter(|&(_, &b)| b == b'\n')
        {
            lines += 1;
            if lines == LINE_LIMIT {
                keep = i + 1;
                break;
            }
        }
        buf.extend_from_slice(&chunk[..keep]);
        if lines >= LINE_LIMIT || buf.len() >= OUTPUT_LIMIT {
            return (buf, true, Ok(()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn output_is_cut_at_the_byte_or_line_limit() {
        let (buf, cut, _) = read_limited(&mut Cursor::new(b"a\nb\n".to_vec()));
        assert_eq!((buf.as_slice(), cut), (&b"a\nb\n"[..], false));

        let many = b"y\n".repeat(LINE_LIMIT + 10);
        let (buf, cut, _) = read_limited(&mut Cursor::new(many));
        assert!(cut);
        assert_eq!(buf, b"y\n".repeat(LINE_LIMIT));

        let long = vec![b'x'; 2 * OUTPUT_LIMIT];
        let (buf, cut, _) = read_limited(&mut Cursor::new(long));
        assert!(cut);
        assert_eq!(buf.len(), OUTPUT_LIMIT);
    }
}
