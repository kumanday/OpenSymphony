//! Job ownership is established while the child is suspended, before it can spawn descendants.
use std::{io, process::ExitStatus, time::Duration};

use process_wrap::tokio::{ChildWrapper, CommandWrap, JobObject, KillOnDrop};
use tokio::process::{ChildStderr, ChildStdin, ChildStdout, Command};

pub(super) struct WindowsChild {
    inner: Box<dyn ChildWrapper>,
    pub(super) stdin: Option<ChildStdin>,
    pub(super) stdout: Option<ChildStdout>,
    pub(super) stderr: Option<ChildStderr>,
}

impl WindowsChild {
    pub(super) fn spawn(command: Command) -> io::Result<Self> {
        let mut inner = CommandWrap::from(command)
            .wrap(KillOnDrop)
            .wrap(JobObject)
            .spawn()?;
        Ok(Self {
            stdin: inner.stdin().take(),
            stdout: inner.stdout().take(),
            stderr: inner.stderr().take(),
            inner,
        })
    }

    pub(super) fn id(&self) -> Option<u32> {
        self.inner.id()
    }

    pub(super) fn start_kill(&mut self) -> io::Result<()> {
        // Direct TerminateJobObject: no external taskkill process or unbounded wait.
        self.inner.start_kill()
    }

    pub(super) async fn wait(&mut self) -> io::Result<ExitStatus> {
        // Avoid the wrapper's blocking job-completion waiter: timeout/drop must
        // release job ownership immediately, including when the parent has exited.
        loop {
            if let Some(status) = self.inner.try_wait()? {
                return Ok(status);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{path::Path, process::Stdio};
    use tokio::time::{sleep, timeout};

    async fn spawn_tree(root: &Path, parent_exits: bool) -> (WindowsChild, u32) {
        let mut command = Command::new("python");
        command
            .arg("-c")
            .arg("import pathlib, subprocess, sys, time; child=subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(120)']); pathlib.Path('descendant.pid').write_text(str(child.pid)); time.sleep(0 if sys.argv[1] == 'exit' else 120)")
            .arg(if parent_exits { "exit" } else { "hang" })
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = WindowsChild::spawn(command).expect("job spawn");
        assert!(child.id().is_some());
        assert!(child.stdin.is_some() && child.stdout.is_some() && child.stderr.is_some());
        let pid = timeout(Duration::from_secs(10), async {
            loop {
                if let Ok(pid) = std::fs::read_to_string(root.join("descendant.pid"))
                    && let Ok(pid) = pid.parse::<u32>()
                {
                    break pid;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("descendant started");
        (child, pid)
    }

    async fn assert_dead(pid: u32) {
        let mut probe = Command::new("powershell.exe");
        probe
            .args(["-NoProfile", "-NonInteractive", "-Command"])
            .arg(format!(
                "for ($i=0; $i -lt 200; $i++) {{ if (-not (Get-Process -Id {pid} -ErrorAction SilentlyContinue)) {{ exit 0 }}; Start-Sleep -Milliseconds 25 }}; exit 1"
            ))
            .kill_on_drop(true);
        let status = timeout(Duration::from_secs(10), probe.status())
            .await
            .expect("bounded probe")
            .expect("process probe");
        assert!(status.success(), "descendant {pid} survived");
    }

    #[tokio::test]
    async fn windows_job_teardown_terminates_descendants() {
        let root = tempfile::tempdir().expect("temp");
        let (mut child, pid) = spawn_tree(root.path(), false).await;
        child.start_kill().expect("terminate job");
        timeout(Duration::from_secs(5), child.wait())
            .await
            .expect("reap deadline")
            .expect("reap");
        drop(child);
        assert_dead(pid).await;
    }

    #[tokio::test]
    async fn windows_job_survives_parent_exit_until_owner_drops() {
        let root = tempfile::tempdir().expect("temp");
        let (mut child, pid) = spawn_tree(root.path(), true).await;
        timeout(Duration::from_secs(5), child.wait())
            .await
            .expect("parent deadline")
            .expect("parent reap");
        drop(child);
        assert_dead(pid).await;
    }

    #[tokio::test]
    async fn windows_job_wait_deadline_remains_cancellation_safe() {
        let root = tempfile::tempdir().expect("temp");
        let (mut child, pid) = spawn_tree(root.path(), false).await;
        assert!(
            timeout(Duration::from_millis(50), child.wait())
                .await
                .is_err()
        );
        drop(child);
        assert_dead(pid).await;
    }

    #[tokio::test]
    async fn windows_job_dropped_future_terminates_descendants() {
        let root = tempfile::tempdir().expect("temp");
        let (mut child, pid) = spawn_tree(root.path(), false).await;
        let (started, active) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            started.send(()).expect("notify owner");
            child.wait().await
        });
        active.await.expect("wait future is active");
        task.abort();
        assert!(task.await.expect_err("aborted future").is_cancelled());
        assert_dead(pid).await;
    }
}
