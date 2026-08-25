//! Spawning and killing downloader processes as a *group* rather than as a
//! single child.
//!
//! yt-dlp is rarely the only process doing the work. On the trim path
//! (`--download-sections`) yt-dlp does not fetch anything itself: it spawns
//! ffmpeg and lets ffmpeg perform the ranged download, and on the untrimmed
//! path it spawns aria2c. Either way the real worker is a *grandchild* of this
//! process, and it inherits the stdout/stderr pipes we handed yt-dlp.
//!
//! Killing only yt-dlp therefore does not stop the download:
//!
//! * ffmpeg keeps running and keeps writing the output file, so a job the UI
//!   reported as cancelled still deposits a file in the user's folder;
//! * ffmpeg still holds the write ends of both pipes, so the runner's
//!   `stdout.read()` and its `stderr` thread's `join()` block until ffmpeg
//!   finishes on its own — one runner thread plus one reader thread linger per
//!   cancel, for as long as the download would have taken.
//!
//! The fix is to put yt-dlp in its own process group at spawn time and signal
//! the whole group on cancel. Every descendant inherits the group id unless it
//! deliberately leaves, so one `killpg` reaches ffmpeg and aria2c too, the
//! pipes close, and both reader threads unblock immediately.

use std::process::Command;

/// Puts the child, and every process it goes on to spawn, in a fresh process
/// group so the group can be signalled as a unit.
///
/// A new group also detaches the child from this process's controlling
/// terminal group, which is a second benefit: a Ctrl-C in a terminal-launched
/// dev build no longer reaches the downloader behind the app's back.
#[cfg(unix)]
pub fn spawn_in_own_process_group(cmd: &mut Command) {
    // `process_group(0)` asks for a new group whose id is the child's own pid.
    // It is applied between fork and exec, so there is no window in which the
    // child exists in the parent's group.
    std::os::unix::process::CommandExt::process_group(cmd, 0);
}

/// No-op on non-Unix targets; see [`kill_tree`] for what that costs.
#[cfg(not(unix))]
pub fn spawn_in_own_process_group(_cmd: &mut Command) {}

/// Kills the child and every descendant it spawned.
///
/// The caller still owns the `Child` and MUST reap it afterwards — this only
/// delivers the signal.
#[cfg(unix)]
pub fn kill_tree(child: &mut std::process::Child) {
    // The group id equals the leader's pid, because `spawn_in_own_process_group`
    // asked for `setpgid(0, 0)`.
    let pid = child.id() as i32;

    // SAFETY: `killpg` only reads the pid; there is no memory involved. The
    // pid may already have been reaped, in which case this returns ESRCH,
    // which is exactly the "nothing left to kill" case and is ignored.
    //
    // The negation is deliberate: `kill(-pgid)` signals the group. `killpg`
    // is used directly here so the sign convention is explicit.
    unsafe {
        libc::killpg(pid, libc::SIGKILL);
    }

    // Belt and braces: if the child somehow left its group (nothing we spawn
    // does), the direct kill still lands. Redundant kills are harmless.
    let _ = child.kill();
}

/// Falls back to killing the direct child only.
///
/// Windows would need a Job Object to do this properly — assigning the child
/// to a job created with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` — which needs a
/// windows-sys dependency and a handle stored alongside every `Child`. Left
/// undone rather than half-done: the grandchild-survives-cancel bug remains on
/// Windows and is called out here so it is not mistaken for fixed.
#[cfg(not(unix))]
pub fn kill_tree(child: &mut std::process::Child) {
    let _ = child.kill();
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::Read;
    use std::process::Stdio;
    use std::sync::mpsc;
    use std::time::Duration;

    /// Spawns `sh -c 'sleep 20 & wait'`: the shell is the child, `sleep` is the
    /// grandchild that inherits the pipe. This is the shape of yt-dlp+ffmpeg.
    fn spawn_parent_with_grandchild() -> std::process::Child {
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c")
            .arg("sleep 20 & wait")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        spawn_in_own_process_group(&mut cmd);
        cmd.spawn().expect("sh should spawn")
    }

    // The whole point: the inherited pipe must reach EOF once the group is
    // killed. Killing only the direct child left the grandchild holding the
    // write end, so the runner's reader blocked for the grandchild's full
    // lifetime — the lingering-thread half of the cancel bug.
    //
    // Read on a worker thread with a bounded wait so a regression fails the
    // test in two seconds instead of hanging the suite for twenty.
    #[test]
    fn killing_the_group_closes_a_pipe_the_grandchild_holds_open() {
        let mut child = spawn_parent_with_grandchild();
        let mut stdout = child.stdout.take().expect("piped");

        kill_tree(&mut child);
        let _ = child.wait();

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = stdout.read_to_end(&mut buf);
            let _ = tx.send(());
        });

        assert!(
            rx.recv_timeout(Duration::from_secs(2)).is_ok(),
            "the inherited pipe should reach EOF once the whole group is killed"
        );
    }

    // A child spawned into its own group must not share ours, or signalling it
    // would signal the app itself.
    #[test]
    fn a_spawned_child_leads_its_own_group() {
        let mut child = spawn_parent_with_grandchild();
        let pid = child.id() as i32;

        // SAFETY: `getpgid` only reads process-table state.
        let group = unsafe { libc::getpgid(pid) };
        assert_eq!(group, pid, "the child should lead a group of its own");
        assert_ne!(group, unsafe { libc::getpgid(0) }, "and not ours");

        kill_tree(&mut child);
        let _ = child.wait();
    }
}
