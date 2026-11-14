//! Starting the daemon: spawning it detached and waiting for its socket to accept.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use interprocess::local_socket::tokio::Stream;

use crate::error::{Error, Result};
use crate::ipc;

/// How long to wait for a freshly spawned daemon to accept connections; a cold model load can
/// take several seconds.
pub(super) const START_TIMEOUT: Duration = Duration::from_secs(15);
/// How often to retry the socket while the daemon starts.
pub(super) const START_POLL: Duration = Duration::from_millis(25);

/// Retries connecting to `socket`, sleeping [`START_POLL`] before each attempt, until it
/// accepts or `timeout` passes.
///
/// # Arguments
/// * `socket` - Path of the daemon's local socket or named pipe.
/// * `timeout` - Total time to keep retrying.
///
/// # Returns
/// The first successful connection.
///
/// # Errors
/// * [`Error::Io`] - Still unreachable at the deadline; keeps the last error's kind and says to
///   run `memhub-mcp daemon` to see why.
pub(super) async fn wait_for_daemon(socket: &Path, timeout: Duration) -> Result<Stream> {
    let deadline = Instant::now() + timeout;
    loop {
        tokio::time::sleep(START_POLL).await;
        match ipc::connect(socket).await {
            Ok(stream) => return Ok(stream),
            Err(e) if Instant::now() >= deadline => {
                return Err(Error::Io(std::io::Error::new(
                    e.kind(),
                    format!(
                        "daemon did not start: cannot connect to {} after {}s: {e} \
                         (run `memhub-mcp daemon` to see why)",
                        socket.display(),
                        timeout.as_secs()
                    ),
                )));
            }
            Err(_) => {}
        }
    }
}

/// Launches `memhub-mcp daemon` from the current executable as a detached background process
/// with null stdio, its own process group, and (on Windows) no console; does not wait for it.
///
/// # Returns
/// `Ok(())` once the child process has been spawned.
///
/// # Errors
/// * [`Error::Io`] - The current executable path cannot be resolved or the process cannot be spawned.
pub(super) fn spawn_daemon() -> Result<()> {
    let mut cmd = Command::new(std::env::current_exe()?);
    cmd.arg("daemon").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    detach(&mut cmd);
    cmd.spawn()?;
    Ok(())
}

/// Puts the child in a new process group so terminal signals sent to the relay (e.g. Ctrl-C)
/// don't reach the daemon.
///
/// # Arguments
/// * `cmd` - The daemon command to configure before spawning.
#[cfg(unix)]
fn detach(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    cmd.process_group(0);
}

/// Sets Windows creation flags so the child has no console, is detached from the relay's
/// console, and runs in its own process group.
///
/// # Arguments
/// * `cmd` - The daemon command to configure before spawning.
#[cfg(windows)]
fn detach(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
}
