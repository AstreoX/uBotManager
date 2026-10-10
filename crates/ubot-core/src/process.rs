//! Child processes: hidden console windows, line streaming and process tree termination.

use std::io::{self, Read};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const DETACHED_PROCESS: u32 = 0x0000_0008;
#[cfg(windows)]
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

/// A command whose console window stays hidden.
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// Starts a GUI program that outlives the manager.
pub fn spawn_detached(mut command: Command) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command.spawn().map(|_| ())
}

/// Runs to completion and returns trimmed stdout, or an error with stderr (or the exit status).
pub fn output_text(mut command: Command) -> Result<String, String> {
    let output = command
        .stdin(Stdio::null())
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if stderr.is_empty() {
            format!("exited with {}", output.status)
        } else {
            stderr
        })
    }
}

/// Kills a process and all of its children (`taskkill /T /F`).
pub fn kill_tree(child: &mut Child) {
    #[cfg(windows)]
    {
        let _ = command("taskkill")
            .args(["/T", "/F", "/PID", &child.id().to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
}

/// Asks a process to close the way closing its window would (`taskkill /PID` without `/F`).
pub fn request_close(pid: u32) -> Result<(), String> {
    #[cfg(windows)]
    {
        output_text({
            let mut taskkill = command("taskkill");
            taskkill.args(["/PID", &pid.to_string()]);
            taskkill
        })
        .map(|_| ())
    }
    #[cfg(not(windows))]
    {
        output_text({
            let mut kill = command("kill");
            kill.args(["-TERM", &pid.to_string()]);
            kill
        })
        .map(|_| ())
    }
}

#[derive(Debug)]
pub enum RunOutcome {
    Exited(ExitStatus),
    Canceled,
}

/// Splits raw output into lines on '\n', '\r\n' and lone '\r' (progress updates). Empty lines are
/// dropped. Bytes after the last break stay in the buffer.
#[derive(Default)]
pub struct LineSplitter {
    buffer: Vec<u8>,
    last_was_cr: bool,
}

impl LineSplitter {
    pub fn push(&mut self, bytes: &[u8], lines: &mut Vec<String>) {
        for &byte in bytes {
            match byte {
                b'\n' if self.last_was_cr => {}
                b'\n' | b'\r' => self.flush(lines),
                _ => self.buffer.push(byte),
            }
            self.last_was_cr = byte == b'\r';
        }
    }

    pub fn finish(&mut self, lines: &mut Vec<String>) {
        self.flush(lines);
    }

    fn flush(&mut self, lines: &mut Vec<String>) {
        let line = String::from_utf8_lossy(&self.buffer).trim_end().to_string();
        self.buffer.clear();
        if !line.is_empty() {
            lines.push(line);
        }
    }
}

fn spawn_reader(
    mut source: impl Read + Send + 'static,
    sender: mpsc::Sender<String>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut splitter = LineSplitter::default();
        let mut chunk = [0u8; 8192];
        let mut lines = Vec::new();
        loop {
            match source.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    splitter.push(&chunk[..read], &mut lines);
                    for line in lines.drain(..) {
                        if sender.send(line).is_err() {
                            return;
                        }
                    }
                }
            }
        }
        splitter.finish(&mut lines);
        for line in lines {
            let _ = sender.send(line);
        }
    })
}

/// What [`run_streaming_events`] reports while a process runs.
pub enum StreamEvent {
    /// A line of stdout or stderr.
    Line(String),
    /// Nothing arrived for a moment (sent about every 50 ms while the process is quiet).
    Idle,
}

/// Runs `command`, passing every stdout and stderr line to `on_line` as it arrives. Polls `cancel`
/// and kills the process tree when it is set.
pub fn run_streaming(
    command: Command,
    cancel: &AtomicBool,
    mut on_line: impl FnMut(String),
) -> io::Result<RunOutcome> {
    run_streaming_events(command, cancel, |event| {
        if let StreamEvent::Line(line) = event {
            on_line(line);
        }
    })
}

/// Like [`run_streaming`], also reporting idle moments so callers can flush buffered output.
pub fn run_streaming_events(
    mut command: Command,
    cancel: &AtomicBool,
    mut on_event: impl FnMut(StreamEvent),
) -> io::Result<RunOutcome> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let (sender, receiver) = mpsc::channel();
    let readers = [
        spawn_reader(
            child.stdout.take().expect("stdout is piped"),
            sender.clone(),
        ),
        spawn_reader(child.stderr.take().expect("stderr is piped"), sender),
    ];

    // The cancel flag is a cheap atomic load and is checked on every iteration, so a process that
    // never stops printing can still be canceled. The child-exit bookkeeping below needs a system
    // call; it runs after a quiet moment and otherwise at most every POLL_INTERVAL.
    const POLL_INTERVAL: Duration = Duration::from_millis(50);
    let mut exited_at: Option<(Instant, ExitStatus)> = None;
    let mut last_poll = Instant::now();
    loop {
        if cancel.load(Ordering::SeqCst) {
            kill_tree(&mut child);
            let _ = child.wait();
            return Ok(RunOutcome::Canceled);
        }
        let quiet = match receiver.recv_timeout(POLL_INTERVAL) {
            Ok(line) => {
                on_event(StreamEvent::Line(line));
                false
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                // Both pipes are closed; the process is done or about to be.
                let status = match exited_at {
                    Some((_, status)) => status,
                    None => child.wait()?,
                };
                for reader in readers {
                    let _ = reader.join();
                }
                return Ok(RunOutcome::Exited(status));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                on_event(StreamEvent::Idle);
                true
            }
        };
        if !quiet && last_poll.elapsed() < POLL_INTERVAL {
            continue;
        }
        last_poll = Instant::now();

        match exited_at {
            None => {
                if let Some(status) = child.try_wait()? {
                    exited_at = Some((Instant::now(), status));
                }
            }
            // A grandchild that inherited the pipes can keep them open; stop waiting for it.
            Some((at, status)) if at.elapsed() > Duration::from_secs(2) => {
                return Ok(RunOutcome::Exited(status))
            }
            Some(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_lines_and_progress() {
        let mut splitter = LineSplitter::default();
        let mut lines = Vec::new();
        splitter.push(b"one\r\ntwo\nRecei", &mut lines);
        splitter.push(b"ving 1%\rReceiving 2%\r\r\n\nlast  ", &mut lines);
        assert_eq!(lines, ["one", "two", "Receiving 1%", "Receiving 2%"]);
        splitter.finish(&mut lines);
        assert_eq!(lines.last().unwrap(), "last");
        let mut invalid = Vec::new();
        let mut splitter = LineSplitter::default();
        splitter.push(b"bad \xFF byte\n", &mut invalid);
        assert_eq!(invalid, ["bad \u{FFFD} byte"]);
    }

    #[test]
    fn streams_output_of_a_process() {
        let mut command = if cfg!(windows) {
            let mut command = super::command("cmd");
            command.args(["/C", "echo first&& echo second 1>&2&& exit 3"]);
            command
        } else {
            let mut command = super::command("sh");
            command.args(["-c", "echo first; echo second 1>&2; exit 3"]);
            command
        };
        command.env("X", "1");
        let cancel = AtomicBool::new(false);
        let mut lines = Vec::new();
        let outcome = run_streaming(command, &cancel, |line| lines.push(line)).unwrap();
        match outcome {
            RunOutcome::Exited(status) => assert_eq!(status.code(), Some(3)),
            RunOutcome::Canceled => panic!("not canceled"),
        }
        lines.sort();
        assert_eq!(lines, ["first", "second"]);
    }

    #[test]
    fn reports_idle_moments_between_lines() {
        let mut command = if cfg!(windows) {
            let mut command = super::command("cmd");
            command.args(["/C", "echo first& ping -n 2 127.0.0.1 >nul& echo second"]);
            command
        } else {
            let mut command = super::command("sh");
            command.args(["-c", "echo first; sleep 1; echo second"]);
            command
        };
        command.env("X", "1");
        let cancel = AtomicBool::new(false);
        let mut events = Vec::new();
        run_streaming_events(command, &cancel, |event| {
            events.push(match event {
                StreamEvent::Line(line) => line,
                StreamEvent::Idle => String::new(),
            })
        })
        .unwrap();
        let first = events.iter().position(|e| e == "first").unwrap();
        let second = events.iter().position(|e| e == "second").unwrap();
        assert!(
            events[first..second]
                .iter()
                .filter(|e| e.is_empty())
                .count()
                >= 5,
            "idle ticks while the process is quiet"
        );
    }

    #[test]
    fn cancels_a_running_process() {
        let command = if cfg!(windows) {
            let mut command = super::command("ping");
            command.args(["-n", "30", "127.0.0.1"]);
            command
        } else {
            let mut command = super::command("sh");
            command.args(["-c", "echo started; sleep 30"]);
            command
        };
        let cancel = AtomicBool::new(false);
        let started = Instant::now();
        let outcome =
            run_streaming(command, &cancel, |_| cancel.store(true, Ordering::SeqCst)).unwrap();
        assert!(matches!(outcome, RunOutcome::Canceled));
        assert!(started.elapsed() < Duration::from_secs(20));
    }

    #[test]
    fn cancels_a_chatty_process() {
        // A process that prints without pause never leaves the receive loop idle; the cancel flag
        // must still be observed (the old loop only looked at it after a quiet 50 ms).
        let command = if cfg!(windows) {
            let mut command = super::command("cmd");
            command.args(["/C", "for /L %i in (1,1,2000000) do @echo line %i"]);
            command
        } else {
            let mut command = super::command("sh");
            command.args(["-c", "yes"]);
            command
        };
        let cancel = AtomicBool::new(false);
        let started = Instant::now();
        let mut seen = 0usize;
        let outcome = run_streaming(command, &cancel, |_| {
            seen += 1;
            if seen == 100 {
                cancel.store(true, Ordering::SeqCst);
            }
        })
        .unwrap();
        assert!(matches!(outcome, RunOutcome::Canceled));
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(seen < 100_000, "stopped early, saw {seen} lines");
    }
}
