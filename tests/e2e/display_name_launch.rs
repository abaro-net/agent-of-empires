//! End-to-end coverage for the title AoE hands the agent as its own session name.
//!
//! The unit tests assert the string `build_host_command` produces. What the pane actually
//! execs is that string run through an ephemeral env-file wrapper and a login shell, which is
//! where quoting breaks, so this drives the whole `aoe add --launch` path with a title that
//! carries a space and an apostrophe and reads the argv the agent was really invoked with.
//! The stub records one argument per line, so a title that arrives split into two arguments
//! fails here rather than looking the same as a correctly quoted one.

use crate::harness::{require_tmux, TuiTestHarness};
use serde_json::Value;
use serial_test::parallel;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Kills the launched tmux session when dropped, matched by the id suffix aoe builds its
/// names from, so a panicking assertion still tears the real session down.
struct TmuxSessionGuard {
    socket: PathBuf,
    id_suffix: String,
}

impl Drop for TmuxSessionGuard {
    fn drop(&mut self) {
        let Ok(list) = Command::new("tmux")
            .arg("-S")
            .arg(&self.socket)
            .args(["list-sessions", "-F", "#{session_name}"])
            .output()
        else {
            return;
        };
        for name in String::from_utf8_lossy(&list.stdout).lines() {
            if name.ends_with(&self.id_suffix) {
                let _ = Command::new("tmux")
                    .arg("-S")
                    .arg(&self.socket)
                    .args(["kill-session", "-t", name])
                    .output();
            }
        }
    }
}

/// A `claude` stub that answers `--help` with the display-name flag, so the launch path's
/// capability probe sees a supporting install, and records the argv of any other invocation.
fn install_claude_stub(h: &mut TuiTestHarness) -> PathBuf {
    let bin = h.install_path_command("claude");
    let record = h.home_path().join("claude.argv");
    let record_str = record.to_string_lossy().to_string();
    assert!(
        !record_str.contains(['"', '$', '`', '\\', '\'']),
        "record path has shell metacharacters: {record_str}"
    );
    let script = format!(
        "#!/bin/sh\n\
         case \"$1\" in --help) printf '  -n, --name <name>  Set a display name\\n'; exit 0;; esac\n\
         printf '%s\\n' \"$@\" > \"{record_str}\"\n\
         exit 0\n"
    );
    std::fs::write(bin.join("claude"), script).expect("write claude stub");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(bin.join("claude"), std::fs::Permissions::from_mode(0o755))
            .expect("chmod claude stub");
    }
    record
}

/// The id of the session titled `title`, so the teardown can match its tmux name.
fn session_id(h: &TuiTestHarness, title: &str) -> String {
    let path = crate::harness::app_dir_in(h.home_path())
        .join("profiles")
        .join("default")
        .join("sessions.json");
    let sessions: Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| panic!("no sessions.json at {} after launch", path.display()));
    sessions
        .as_array()
        .and_then(|arr| arr.iter().find(|s| s["title"].as_str() == Some(title)))
        .and_then(|s| s["id"].as_str())
        .unwrap_or_else(|| panic!("no session titled '{title}' in {}", path.display()))
        .to_string()
}

/// Poll (up to 10s) for the stub to record the launch argv. Hook installation can invoke the
/// same stub first, so this waits for the invocation that carries the flag.
fn wait_for_launch_argv(path: &Path) -> Vec<String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if let Ok(content) = std::fs::read_to_string(path) {
            if content.contains("--name") {
                return content.lines().map(str::to_string).collect();
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the launch never recorded an argv carrying --name at {}",
            path.display()
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

#[test]
#[parallel]
fn a_launch_hands_the_agent_its_title_as_one_argument() {
    require_tmux!();
    let mut h = TuiTestHarness::new("display_name_launch");
    let title = "O'Brien's plan";
    let record = install_claude_stub(&mut h);

    let project = h.project_path();
    let output = h.run_cli(&[
        "add",
        project.to_str().unwrap(),
        "-t",
        title,
        "--tool",
        "claude",
        "--launch",
    ]);
    assert!(
        output.status.success(),
        "aoe add --launch should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let id = session_id(&h, title);
    let _guard = TmuxSessionGuard {
        socket: h.home_path().join("tmux.sock"),
        id_suffix: format!("_{}", &id[..8.min(id.len())]),
    };

    let argv = wait_for_launch_argv(&record);
    let at = argv
        .iter()
        .position(|arg| arg == "--name")
        .unwrap_or_else(|| panic!("no --name in the launch argv: {argv:?}"));
    assert_eq!(
        argv.get(at + 1).map(String::as_str),
        Some(title),
        "the title reaches the agent as a single argument, quoting intact: {argv:?}"
    );
}
