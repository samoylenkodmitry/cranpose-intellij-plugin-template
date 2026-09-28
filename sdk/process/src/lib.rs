//! Own the complete preview lifetime, including compilers and inherited pipes.
//! Unix workers share the SDK runner's group; Windows descendants belong to a Job.
#![cfg_attr(not(windows), forbid(unsafe_code))]

mod capture;
pub use capture::capture;
mod stream;
pub use stream::{Cancellation, ExecutionEvent, execute, execute_observed};
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::cpu_time;

use std::{
    io,
    ops::{Deref, DerefMut},
    process::{Child, Command, ExitStatus},
    time::{Duration, Instant},
};

const OWNER: &str = "CRANPOSE_PLUGIN_PROCESS_OWNER";
const POLL: Duration = Duration::from_millis(10);
const REAP: Duration = Duration::from_millis(500);

/// A child and its descendants. Dropping this handle forcibly stops the owned scope.
/// Use `terminate` on a worker thread to allow a bounded graceful shutdown first.
pub struct Process {
    child: Child,
    cleaned: bool,
    #[cfg(unix)]
    group: bool,
    #[cfg(windows)]
    job: windows::Job,
}

impl Process {
    /// Start a separate preview or command with its own descendant scope.
    pub fn spawn(command: Command) -> io::Result<Self> {
        Self::start(command, false)
    }

    /// Start a compiler inside an SDK-owned runner. On Unix it inherits the
    /// runner's group so the host's force-stop also reaches compiler descendants.
    /// Standalone runners get a separate compiler group instead.
    pub fn spawn_worker(command: Command) -> io::Result<Self> {
        Self::start(command, true)
    }

    fn start(mut command: Command, worker: bool) -> io::Result<Self> {
        #[cfg(unix)]
        let group = {
            use nix::unistd::{getpgrp, getpid};
            use std::os::unix::process::CommandExt;
            let inherited = worker
                && std::env::var_os(OWNER).as_deref() == Some(std::ffi::OsStr::new("1"))
                && getpgrp() == getpid();
            if !inherited {
                command.process_group(0);
            }
            !inherited
        };
        if worker {
            command.env_remove(OWNER);
        } else {
            command.env(OWNER, "1");
        }
        #[cfg(windows)]
        let job = windows::Job::prepare(&mut command)?;
        let child = command.spawn()?;
        let process = Self {
            child,
            cleaned: false,
            #[cfg(unix)]
            group,
            #[cfg(windows)]
            job,
        };
        #[cfg(windows)]
        process.job.attach_and_resume(&process.child)?;
        Ok(process)
    }

    /// Signal now, allow `grace` for orderly exit, then force-stop descendants.
    /// Reaping is bounded to another 500 ms; no UI thread should call this method.
    pub fn terminate(&mut self, grace: Duration) -> io::Result<ExitStatus> {
        if self.cleaned {
            return self.child.try_wait()?.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    "child has not exited after force-stop",
                )
            });
        }
        #[cfg(unix)]
        if !grace.is_zero() {
            self.signal(nix::sys::signal::Signal::SIGTERM);
            let deadline = Instant::now() + grace;
            while self.child.try_wait()?.is_none() && Instant::now() < deadline {
                std::thread::sleep(POLL);
            }
        }
        #[cfg(not(unix))]
        let _ = grace;
        self.force();
        let deadline = Instant::now() + REAP;
        loop {
            if let Some(status) = self.child.try_wait()? {
                return Ok(status);
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "child did not exit after force-stop",
                ));
            }
            std::thread::sleep(POLL);
        }
    }

    /// After stopping a child, confirm that no processes still use its scope.
    /// Shared Unix workers exclude this runner, which still holds its cache lease.
    /// This only observes processes. A false result means callers must abandon
    /// reusable resources instead of making them available to another session.
    pub fn wait_for_tree_exit(&mut self, timeout: Duration) -> io::Result<bool> {
        let deadline = Instant::now() + timeout;
        loop {
            if self.child.try_wait()?.is_some() && self.scope_empty()? {
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            std::thread::sleep(POLL.min(deadline.saturating_duration_since(Instant::now())));
        }
    }
    fn scope_empty(&self) -> io::Result<bool> {
        #[cfg(target_os = "linux")]
        {
            let group = if self.group {
                self.child.id()
            } else {
                nix::unistd::getpgrp().as_raw() as u32
            };
            if nix::sys::signal::killpg(nix::unistd::Pid::from_raw(group as i32), None)
                == Err(nix::errno::Errno::ESRCH)
            {
                return Ok(true);
            }
            for entry in std::fs::read_dir("/proc")? {
                let entry = entry?;
                let Some(pid) = entry
                    .file_name()
                    .to_str()
                    .and_then(|s| s.parse::<u32>().ok())
                else {
                    continue;
                };
                if !self.group && pid == std::process::id() {
                    continue;
                }
                let stat = match std::fs::read_to_string(entry.path().join("stat")) {
                    Ok(stat) => stat,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(error),
                };
                // comm is parenthesized and may itself contain spaces or ')'.
                let Some((_, tail)) = stat.rsplit_once(") ") else {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Invalid process stat",
                    ));
                };
                let mut fields = tail.split_whitespace();
                let state = fields.next();
                let process_group = fields.nth(1).and_then(|s| s.parse::<u32>().ok());
                if process_group == Some(group) && !matches!(state, Some("Z" | "X")) {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        #[cfg(target_os = "macos")]
        {
            use libproc::processes::{ProcFilter, pids_by_type};
            let group = if self.group {
                self.child.id()
            } else {
                nix::unistd::getpgrp().as_raw() as u32
            };
            let owner = std::process::id();
            let pids = match pids_by_type(ProcFilter::ByProgramGroup { pgrpid: group }) {
                Ok(pids) => pids,
                Err(error) => {
                    // macOS libproc may report stale errno when the last process
                    // exits during enumeration. Confirm absence independently;
                    // permission errors or an existing group stay conservative.
                    if nix::sys::signal::killpg(nix::unistd::Pid::from_raw(group as i32), None)
                        == Err(nix::errno::Errno::ESRCH)
                    {
                        return Ok(true);
                    }
                    return Err(error);
                }
            };
            Ok(pids
                .into_iter()
                .all(|pid| pid == 0 || (!self.group && pid == owner) || !is_running(pid)))
        }
        #[cfg(windows)]
        {
            self.job.is_empty()
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
        {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "process scope observation is unavailable",
            ))
        }
    }

    #[cfg(unix)]
    fn signal(&self, signal: nix::sys::signal::Signal) {
        use nix::{
            sys::signal::{kill, killpg},
            unistd::Pid,
        };
        let pid = Pid::from_raw(self.child.id() as i32);
        let _ = if self.group {
            killpg(pid, signal)
        } else {
            kill(pid, signal)
        };
    }

    fn force(&mut self) {
        if self.cleaned {
            return;
        }
        #[cfg(unix)]
        self.signal(nix::sys::signal::Signal::SIGKILL);
        #[cfg(windows)]
        self.job.terminate();
        let _ = self.child.kill();
        self.cleaned = true;
    }
}

impl Deref for Process {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.child
    }
}
impl DerefMut for Process {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.child
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.terminate(Duration::ZERO);
    }
}

/// Diagnostic liveness check for shutdown regression tests and development tools.
/// A zombie has exited and is not considered running. Never use this to signal a PID.
pub fn is_running(pid: u32) -> bool {
    #[cfg(unix)]
    {
        #[cfg(target_os = "linux")]
        if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat"))
            && stat
                .rsplit_once(") ")
                .is_some_and(|(_, tail)| tail.starts_with('Z'))
        {
            return false;
        }
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), None)
            != Err(nix::errno::Errno::ESRCH)
    }
    #[cfg(windows)]
    {
        windows::is_running(pid)
    }
}
