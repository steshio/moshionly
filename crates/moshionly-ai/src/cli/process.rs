//! Running an agent CLI: stream its output line by line, stop it on request,
//! and never leave a process behind.
//!
//! On Unix the child gets its own process group. Stopping it signals the
//! whole group, so anything the agent started (a shell command, a helper)
//! goes with it. The group is also killed if the future running the child is
//! dropped.

use std::ffi::OsString;
use std::future::Future;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

use crate::error::AiError;

/// How long a child gets to exit after a polite request before it is killed.
const TERMINATE_GRACE: Duration = Duration::from_millis(1500);

/// The most stderr kept for error messages. Agents can be chatty; only the
/// end matters.
const STDERR_TAIL_BYTES: usize = 16 * 1024;

/// A command line to run.
#[derive(Debug, Clone)]
pub(crate) struct CliCommand {
    /// The executable.
    pub program: PathBuf,
    /// Its arguments.
    pub args: Vec<OsString>,
    /// The directory to run in.
    pub cwd: Option<PathBuf>,
    /// Environment variables to set on top of the inherited environment.
    pub env: Vec<(OsString, OsString)>,
    /// Text written to the child's stdin, which is then closed. `None` gives
    /// the child an empty stdin so that it can never wait for a terminal.
    pub stdin: Option<String>,
    /// After the caller reports that the output is complete, how long the
    /// child may take to exit on its own before it is stopped.
    pub exit_grace: Duration,
}

impl CliCommand {
    /// A command with no arguments yet.
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
            stdin: None,
            exit_grace: Duration::from_secs(5),
        }
    }

    /// Appends one argument.
    pub fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Appends several arguments.
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// The arguments as text, for tests and debugging.
    #[cfg(test)]
    pub fn args_lossy(&self) -> Vec<String> {
        self.args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }
}

/// What the caller wants after seeing a line of output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LineAction {
    /// Keep reading.
    Continue,
    /// The output is complete. The child is given a moment to exit and is
    /// stopped if it lingers.
    Finish,
}

/// How a child ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProcessExit {
    /// It ran to the end.
    Exited {
        /// The exit code, or `None` when a signal ended it.
        code: Option<i32>,
        /// The last part of what it wrote to stderr.
        stderr: String,
    },
    /// It was stopped because `stop` completed.
    Stopped,
}

/// Runs `command`, calling `on_line` for every line it writes to stdout.
///
/// When `stop` completes the child and everything it started are terminated
/// and [`ProcessExit::Stopped`] is returned. Dropping the returned future has
/// the same effect on the processes.
pub(crate) async fn run_streaming<S, F>(
    command: CliCommand,
    stop: S,
    mut on_line: F,
) -> Result<ProcessExit, AiError>
where
    S: Future<Output = ()>,
    F: FnMut(&str) -> LineAction,
{
    let mut child = spawn(&command)?;
    let group = ProcessGroup::of(&child);

    if let Some(mut stdin) = child.stdin.take() {
        let input = command.stdin.clone().unwrap_or_default();
        tokio::spawn(async move {
            // A child that exits early closes the pipe; that is not an error
            // worth reporting, its exit status tells the story.
            let _ = stdin.write_all(input.as_bytes()).await;
            let _ = stdin.shutdown().await;
        });
    }

    let stderr_task = child.stderr.take().map(|stderr| {
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr);
            let mut tail = Vec::new();
            let mut chunk = [0u8; 4096];
            while let Ok(read) = reader.read(&mut chunk).await {
                if read == 0 {
                    break;
                }
                tail.extend_from_slice(&chunk[..read]);
                if tail.len() > STDERR_TAIL_BYTES * 2 {
                    tail.drain(..tail.len() - STDERR_TAIL_BYTES);
                }
            }
            if tail.len() > STDERR_TAIL_BYTES {
                tail.drain(..tail.len() - STDERR_TAIL_BYTES);
            }
            String::from_utf8_lossy(&tail).trim().to_owned()
        })
    });

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AiError::other("the agent's output could not be captured"))?;
    let mut reader = BufReader::new(stdout);
    let mut line = Vec::new();
    let mut output_complete = false;

    tokio::pin!(stop);
    loop {
        line.clear();
        tokio::select! {
            biased;
            () = &mut stop => {
                terminate(&mut child, &group).await;
                return Ok(ProcessExit::Stopped);
            }
            read = reader.read_until(b'\n', &mut line) => match read {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let text = String::from_utf8_lossy(&line);
                    let text = text.trim_end_matches(['\n', '\r']);
                    if !text.is_empty() && on_line(text) == LineAction::Finish {
                        output_complete = true;
                        break;
                    }
                }
            },
        }
    }

    let waited = if output_complete {
        match tokio::time::timeout(command.exit_grace, child.wait()).await {
            Ok(status) => status,
            Err(_) => {
                // The agent has said everything but is not exiting. Its answer
                // is already delivered, so stop it and report a normal end.
                terminate(&mut child, &group).await;
                let stderr = collect(stderr_task).await;
                return Ok(ProcessExit::Exited {
                    code: Some(0),
                    stderr,
                });
            }
        }
    } else {
        tokio::select! {
            biased;
            () = &mut stop => {
                terminate(&mut child, &group).await;
                return Ok(ProcessExit::Stopped);
            }
            status = child.wait() => status,
        }
    };

    group.disarm();
    let status =
        waited.map_err(|error| AiError::other(format!("waiting for the agent: {error}")))?;
    let stderr = collect(stderr_task).await;
    Ok(ProcessExit::Exited {
        code: status.code(),
        stderr,
    })
}

/// Runs `command` to completion and returns its stdout, for short helper
/// invocations such as `--version`. The child is killed when `limit` passes.
pub(crate) async fn run_to_string(
    command: CliCommand,
    limit: Duration,
) -> Result<(Option<i32>, String, String), AiError> {
    let mut stdout = String::new();
    let exit = run_streaming(command, tokio::time::sleep(limit), |line| {
        stdout.push_str(line);
        stdout.push('\n');
        LineAction::Continue
    })
    .await?;
    match exit {
        ProcessExit::Exited { code, stderr } => Ok((code, stdout, stderr)),
        ProcessExit::Stopped => Err(AiError::other("the command did not answer in time")),
    }
}

fn spawn(command: &CliCommand) -> Result<Child, AiError> {
    let mut cmd = Command::new(&command.program);
    cmd.args(&command.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(cwd) = &command.cwd {
        cmd.current_dir(cwd);
    }
    for (key, value) in &command.env {
        cmd.env(key, value);
    }
    // A group of its own, so that signals reach the agent's children too and
    // a Ctrl-C in a terminal running the app does not reach the agent twice.
    #[cfg(unix)]
    cmd.process_group(0);

    cmd.spawn().map_err(|error| {
        let program = command
            .program
            .file_name()
            .unwrap_or(command.program.as_os_str())
            .to_string_lossy()
            .into_owned();
        if error.kind() == std::io::ErrorKind::NotFound {
            AiError::NotInstalled { program }
        } else {
            AiError::other(format!("could not start {program}: {error}"))
        }
    })
}

async fn collect(task: Option<tokio::task::JoinHandle<String>>) -> String {
    match task {
        // The pipe can outlive the child if a grandchild inherited it, so the
        // wait is bounded.
        Some(task) => tokio::time::timeout(Duration::from_millis(500), task)
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default(),
        None => String::new(),
    }
}

/// Stops the child: a polite request first, so an agent can save its session,
/// then a kill for anything in the group that is still alive.
async fn terminate(child: &mut Child, group: &ProcessGroup) {
    group.signal_terminate();
    if tokio::time::timeout(TERMINATE_GRACE, child.wait())
        .await
        .is_err()
    {
        let _ = child.start_kill();
    }
    group.signal_kill();
    let _ = child.wait().await;
    group.disarm();
}

/// The process group of a running child. Dropping it while armed kills the
/// group, which covers a future that is dropped half way through.
struct ProcessGroup {
    #[cfg(unix)]
    pgid: Option<nix::unistd::Pid>,
    #[cfg(unix)]
    armed: std::sync::atomic::AtomicBool,
}

#[cfg(unix)]
impl ProcessGroup {
    fn of(child: &Child) -> Self {
        let pgid = child
            .id()
            .and_then(|id| i32::try_from(id).ok())
            .map(nix::unistd::Pid::from_raw);
        Self {
            pgid,
            armed: std::sync::atomic::AtomicBool::new(true),
        }
    }

    fn signal(&self, signal: nix::sys::signal::Signal) {
        if let Some(pgid) = self.pgid {
            // The group may already be gone; that is the outcome we want.
            let _ = nix::sys::signal::killpg(pgid, signal);
        }
    }

    fn signal_terminate(&self) {
        self.signal(nix::sys::signal::Signal::SIGTERM);
    }

    fn signal_kill(&self) {
        self.signal(nix::sys::signal::Signal::SIGKILL);
    }

    fn disarm(&self) {
        self.armed
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(unix)]
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        if self.armed.load(std::sync::atomic::Ordering::Relaxed) {
            self.signal_kill();
        }
    }
}

// TODO: on Windows only the direct child is stopped (through tokio's
// kill-on-drop). Put the child in a job object so its descendants go too.
#[cfg(not(unix))]
impl ProcessGroup {
    fn of(_child: &Child) -> Self {
        Self {}
    }

    fn signal_terminate(&self) {}

    fn signal_kill(&self) {}

    fn disarm(&self) {}
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::Mutex;
    use tokio_util::sync::CancellationToken;

    fn sh(script: &str) -> CliCommand {
        CliCommand::new("/bin/sh").arg("-c").arg(script)
    }

    fn is_alive(pid: i32) -> bool {
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok()
    }

    async fn wait_until_dead(pid: i32) -> bool {
        for _ in 0..100 {
            if !is_alive(pid) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        false
    }

    #[tokio::test]
    async fn delivers_lines_in_order_with_exit_code_and_stderr() {
        let mut lines = Vec::new();
        let exit = run_streaming(
            sh("echo one; echo two; echo oops >&2; exit 3"),
            std::future::pending(),
            |line| {
                lines.push(line.to_owned());
                LineAction::Continue
            },
        )
        .await
        .unwrap();
        assert_eq!(lines, ["one", "two"]);
        assert_eq!(
            exit,
            ProcessExit::Exited {
                code: Some(3),
                stderr: "oops".to_owned()
            }
        );
    }

    #[tokio::test]
    async fn writes_stdin_and_closes_it() {
        let mut command = CliCommand::new("/bin/cat");
        command.stdin = Some("first\nsecond".to_owned());
        let mut lines = Vec::new();
        run_streaming(command, std::future::pending(), |line| {
            lines.push(line.to_owned());
            LineAction::Continue
        })
        .await
        .unwrap();
        assert_eq!(lines, ["first", "second"]);
    }

    #[tokio::test]
    async fn stdin_is_empty_when_none_is_given() {
        // `cat` would wait for ever on an open terminal or pipe.
        let exit = run_streaming(CliCommand::new("/bin/cat"), std::future::pending(), |_| {
            LineAction::Continue
        })
        .await
        .unwrap();
        assert!(matches!(exit, ProcessExit::Exited { code: Some(0), .. }));
    }

    #[tokio::test]
    async fn a_missing_program_is_reported_as_not_installed() {
        let error = run_streaming(
            CliCommand::new("/nonexistent/moshionly-test-agent"),
            std::future::pending(),
            |_| LineAction::Continue,
        )
        .await
        .unwrap_err();
        assert_eq!(
            error,
            AiError::NotInstalled {
                program: "moshionly-test-agent".to_owned()
            }
        );
    }

    #[tokio::test]
    async fn stopping_kills_the_child_and_its_descendants() {
        let cancel = CancellationToken::new();
        let pids = Arc::new(Mutex::new(Vec::<i32>::new()));
        let seen = pids.clone();
        let trigger = cancel.clone();
        // The shell starts a grandchild, prints both pids, then waits.
        let exit = run_streaming(
            sh("sleep 300 & echo $$; echo $!; wait"),
            cancel.cancelled(),
            move |line| {
                let mut seen = seen.lock().unwrap();
                seen.push(line.trim().parse().unwrap());
                if seen.len() == 2 {
                    trigger.cancel();
                }
                LineAction::Continue
            },
        )
        .await
        .unwrap();
        assert_eq!(exit, ProcessExit::Stopped);
        let pids = pids.lock().unwrap().clone();
        assert_eq!(pids.len(), 2);
        for pid in pids {
            assert!(wait_until_dead(pid).await, "process {pid} survived");
        }
    }

    #[tokio::test]
    async fn stopping_kills_a_child_that_ignores_the_polite_request() {
        let cancel = CancellationToken::new();
        let pid = Arc::new(Mutex::new(None::<i32>));
        let seen = pid.clone();
        let trigger = cancel.clone();
        let started = std::time::Instant::now();
        let exit = run_streaming(
            sh("trap '' TERM; echo $$; while true; do sleep 1; done"),
            cancel.cancelled(),
            move |line| {
                *seen.lock().unwrap() = line.trim().parse().ok();
                trigger.cancel();
                LineAction::Continue
            },
        )
        .await
        .unwrap();
        assert_eq!(exit, ProcessExit::Stopped);
        assert!(started.elapsed() < Duration::from_secs(10));
        let pid = pid.lock().unwrap().unwrap();
        assert!(wait_until_dead(pid).await, "process {pid} survived");
    }

    #[tokio::test]
    async fn dropping_the_future_kills_the_group() {
        let (pid_sender, pid_receiver) = tokio::sync::oneshot::channel::<i32>();
        let mut pid_sender = Some(pid_sender);
        let run = run_streaming(
            sh("sleep 300 & echo $!; wait"),
            std::future::pending(),
            move |line| {
                if let Some(sender) = pid_sender.take() {
                    let _ = sender.send(line.trim().parse().unwrap());
                }
                LineAction::Continue
            },
        );
        let task = tokio::spawn(run);
        let grandchild = pid_receiver.await.unwrap();
        assert!(is_alive(grandchild));
        task.abort();
        let _ = task.await;
        assert!(
            wait_until_dead(grandchild).await,
            "process {grandchild} survived"
        );
    }

    #[tokio::test]
    async fn a_child_that_lingers_after_its_output_is_complete_is_stopped() {
        let mut command = sh("echo done; sleep 300");
        command.exit_grace = Duration::from_millis(100);
        let started = std::time::Instant::now();
        let exit = run_streaming(command, std::future::pending(), |line| {
            assert_eq!(line, "done");
            LineAction::Finish
        })
        .await
        .unwrap();
        assert!(matches!(exit, ProcessExit::Exited { code: Some(0), .. }));
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[tokio::test]
    async fn run_to_string_times_out() {
        let error = run_to_string(sh("sleep 300"), Duration::from_millis(100))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("in time"));
    }

    #[tokio::test]
    async fn run_to_string_collects_output() {
        let (code, stdout, stderr) =
            run_to_string(sh("echo 1.2.3; echo note >&2"), Duration::from_secs(10))
                .await
                .unwrap();
        assert_eq!(code, Some(0));
        assert_eq!(stdout.trim(), "1.2.3");
        assert_eq!(stderr, "note");
    }

    #[tokio::test]
    async fn output_that_is_not_utf8_does_not_fail_the_run() {
        let mut lines = Vec::new();
        run_streaming(
            sh("printf 'ok\\377\\n'; echo next"),
            std::future::pending(),
            |line| {
                lines.push(line.to_owned());
                LineAction::Continue
            },
        )
        .await
        .unwrap();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("ok"));
        assert_eq!(lines[1], "next");
    }
}
