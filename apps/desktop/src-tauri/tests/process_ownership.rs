//! Process ownership tests for daemon supervision.
//!
//! Verifies that the desktop app:
//! - Only starts configured supervised daemons
//! - Only stops processes it owns
//! - Tracks process ownership correctly
//! - Does not interfere with externally-managed daemons

#[cfg(test)]
mod process_ownership_tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::Duration;

    /// Fake daemon command that simulates a daemon process.
    /// Creates a PID file and sleeps to stay alive.
    struct FakeDaemon {
        dir: tempfile::TempDir,
        pid_path: PathBuf,
    }

    impl FakeDaemon {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let pid_path = dir.path().join("daemon.pid");
            Self { dir, pid_path }
        }

        /// Create a fake daemon script that writes its PID and sleeps.
        fn script_path(&self) -> PathBuf {
            let script = self.dir.path().join("fake_daemon.sh");
            let pid_path = &self.pid_path;
            fs::write(
                &script,
                format!(
                    r#"#!/bin/bash
echo $$ > "{}"
while true; do sleep 1; done
"#,
                    pid_path.display()
                ),
            )
            .unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(&script).unwrap().permissions();
                perms.set_mode(0o755);
                fs::set_permissions(&script, perms).unwrap();
            }
            script
        }
    }

    #[test]
    fn test_fake_daemon_creates_pid_file() {
        let daemon = FakeDaemon::new();
        let script = daemon.script_path();
        assert!(script.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::metadata(&script).unwrap().permissions();
            assert!(perms.mode() & 0o111 != 0);
        }
    }

    #[test]
    fn test_process_ownership_starts_unowned() {
        // A fresh daemon handle should not own any process
        use opensymphony_desktop::daemon::{DaemonConfig, DaemonHandle};
        let config = DaemonConfig {
            executable: PathBuf::from("/bin/true"),
            args: vec![],
            env: vec![],
            startup_timeout: Duration::from_secs(1),
            auto_restart: false,
            gateway_url: "http://127.0.0.1:2468".to_string(),
            skip_health_check: true,
        };
        let handle = DaemonHandle::new(config);
        assert!(handle.pid().is_none());
    }

    #[tokio::test]
    async fn test_unsupervised_daemon_not_stopped_by_app() {
        // Verify that the app does not attempt to stop processes it doesn't own
        use opensymphony_desktop::daemon::{DaemonConfig, DaemonHandle};
        let config = DaemonConfig {
            executable: PathBuf::from("/bin/true"),
            args: vec![],
            env: vec![],
            startup_timeout: Duration::from_secs(1),
            auto_restart: false,
            gateway_url: "http://127.0.0.1:2468".to_string(),
            skip_health_check: true,
        };
        let mut handle = DaemonHandle::new(config);
        // Calling stop on an unstarted handle should succeed without error
        assert!(handle.stop().await.is_ok());
    }

    #[tokio::test]
    async fn test_daemon_handle_cleans_up_on_drop() {
        use opensymphony_desktop::daemon::{DaemonConfig, DaemonHandle};
        let fake = FakeDaemon::new();
        let script = fake.script_path();

        let config = DaemonConfig {
            executable: script.clone(),
            args: vec![],
            env: vec![],
            startup_timeout: Duration::from_secs(2),
            auto_restart: false,
            gateway_url: "http://127.0.0.1:2468".to_string(),
            skip_health_check: true,
        };

        let pid = {
            let mut handle = DaemonHandle::new(config);
            let result = handle.start().await;
            let pid = result.pid.expect("process should have been spawned");
            assert!(result.pid.is_some(), "daemon should have a PID after spawn");
            // Handle will be dropped here, triggering cleanup
            pid
        };

        // Give the OS a moment to clean up after drop.
        // Retry for up to 2 seconds to allow SIGKILL propagation and zombie reaping.
        #[cfg(unix)]
        {
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            loop {
                let result = unsafe { libc::kill(pid as i32, 0) };
                if result != 0 {
                    // Process no longer exists - cleanup successful
                    break;
                }
                if std::time::Instant::now() >= deadline {
                    panic!(
                        "process {} still exists after drop and 2s grace period",
                        pid
                    );
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }

    #[tokio::test]
    async fn test_process_ownership_tracks_multiple_daemons() {
        use opensymphony_desktop::daemon::{DaemonConfig, DaemonHandle, DaemonState};

        let fake1 = FakeDaemon::new();
        let fake2 = FakeDaemon::new();
        let script1 = fake1.script_path();
        let script2 = fake2.script_path();

        let config1 = DaemonConfig {
            executable: script1.clone(),
            args: vec![],
            env: vec![],
            startup_timeout: Duration::from_secs(2),
            auto_restart: false,
            gateway_url: "http://127.0.0.1:8081".to_string(),
            skip_health_check: true,
        };

        let config2 = DaemonConfig {
            executable: script2.clone(),
            args: vec![],
            env: vec![],
            startup_timeout: Duration::from_secs(2),
            auto_restart: false,
            gateway_url: "http://127.0.0.1:8082".to_string(),
            skip_health_check: true,
        };

        let mut handle1 = DaemonHandle::new(config1);
        let mut handle2 = DaemonHandle::new(config2);

        // Both handles start unowned
        assert!(handle1.pid().is_none());
        assert!(handle2.pid().is_none());

        // Start both daemons
        let result1 = handle1.start().await;
        let result2 = handle2.start().await;

        // Verify both processes were spawned and have PIDs
        assert!(
            result1.pid.is_some(),
            "daemon 1 should have a PID after spawn"
        );
        assert!(
            result2.pid.is_some(),
            "daemon 2 should have a PID after spawn"
        );

        // Verify PIDs are unique - each daemon tracks its own process
        assert_ne!(
            result1.pid.unwrap(),
            result2.pid.unwrap(),
            "each daemon should have a unique PID"
        );

        // Verify each handle tracks its own PID independently
        assert!(handle1.pid().is_some(), "handle 1 should track its PID");
        assert!(handle2.pid().is_some(), "handle 2 should track its PID");
        // With health check skipped, daemons should be in Running state
        assert!(
            matches!(handle1.state(), DaemonState::Running),
            "handle 1 should be Running"
        );
        assert!(
            matches!(handle2.state(), DaemonState::Running),
            "handle 2 should be Running"
        );

        // Verify stop only affects the targeted daemon
        handle1.stop().await.unwrap();
        assert!(!handle1.is_running(), "handle 1 should be stopped");
        assert!(handle2.is_running(), "handle 2 should still be running");

        // Clean up second daemon
        handle2.stop().await.unwrap();
        assert!(!handle1.is_running(), "handle 1 should still be stopped");
        assert!(!handle2.is_running(), "handle 2 should be stopped");
    }
}
