//! Typing into the home list with `typing_compose_window_ms` set opens the
//! send-message dialog instead of firing shortcuts.

use std::time::Duration;

use agent_of_empires::tmux::Session;
use serial_test::parallel;

use crate::harness::{parse_session_id, require_tmux, TuiTestHarness};

fn add_running_session(h: &TuiTestHarness, title: &str) {
    let project = h.project_path();
    let id = parse_session_id(&h.run_cli_ok(&["add", project.to_str().unwrap(), "-t", title]));
    h.tmux_new_detached(&Session::generate_name(&id, title), "sleep 600");
}

#[test]
#[parallel]
fn typing_in_the_list_opens_compose_and_drops_the_trailing_enter() {
    require_tmux!();
    let mut h = TuiTestHarness::new("typing_compose");
    h.append_config("[session]\ntyping_compose_window_ms = 1500");
    add_running_session(&h, "TypeMe");

    h.spawn_tui();
    h.wait_for_ready();
    h.wait_for("TypeMe");

    // Unfenced, with gaps well past the paste threshold, so the keys arrive
    // as typing: `z` snoozes, `q` asks to quit, `n` opens New Session. A
    // pointer drifting mid-word must not split the burst.
    for key in ["z", "q", "n", "Enter"] {
        h.send_keys_unfenced(key);
        if key == "z" {
            h.send_mouse_move_unfenced(10, 10);
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    h.wait_for(" > TypeMe ");
    h.assert_screen_contains("zqn");
    h.assert_screen_not_contains("New Session");
    h.assert_screen_not_contains("Quit Agent of Empires");

    h.send_keys("Escape");
    h.wait_for_absent(" > TypeMe ", Duration::from_secs(10));
    h.send_keys("n");
    h.wait_for(" New Session ");
}

/// A held key is navigation: it is replayed without waiting out the window,
/// so the compose key pressed next opens on the row it moved to.
#[test]
#[parallel]
fn a_held_key_navigates_without_waiting_out_the_window() {
    require_tmux!();
    let mut h = TuiTestHarness::new("typing_compose_held");
    h.append_config("[session]\ntyping_compose_window_ms = 5000");
    add_running_session(&h, "First");
    add_running_session(&h, "Second");

    h.spawn_tui();
    h.wait_for_ready();
    h.wait_for("Second");
    h.send_keys("m");
    let start = h.capture_screen();
    let (from, to) = if start.contains(" > First ") {
        ("First", "Second")
    } else {
        assert!(start.contains(" > Second "), "{start}");
        ("Second", "First")
    };
    h.send_keys("Escape");
    h.wait_for_absent(&format!(" > {from} "), Duration::from_secs(10));

    // The cursor starts on the top row, so a held `j` moves to the other one.
    for _ in 0..3 {
        h.send_keys_unfenced("j");
        std::thread::sleep(Duration::from_millis(40));
    }
    h.send_keys("m");
    h.wait_for(&format!(" > {to} "));
}

/// A doubled first letter is not a held key, whether pasted or typed.
#[test]
#[parallel]
fn a_doubled_first_letter_stays_in_the_burst() {
    require_tmux!();
    let mut h = TuiTestHarness::new("typing_compose_doubled");
    h.append_config("[session]\ntyping_compose_window_ms = 1500");
    add_running_session(&h, "TypeMe");

    h.spawn_tui();
    h.wait_for_ready();
    h.wait_for("TypeMe");

    h.type_text("eeexy");
    h.wait_for(" > TypeMe ");
    h.assert_screen_contains("eeexy");
    h.send_keys("Escape");
    h.wait_for_absent(" > TypeMe ", Duration::from_secs(10));

    for key in ["l", "l", "z", "q"] {
        h.send_keys_unfenced(key);
        std::thread::sleep(Duration::from_millis(40));
    }
    h.wait_for(" > TypeMe ");
    h.assert_screen_contains("llzq");
}

/// Pointer motion does not extend the window: a key after a longer pause
/// starts over, so the first one has already run as a shortcut.
#[test]
#[parallel]
fn pointer_motion_does_not_hold_the_window_open() {
    require_tmux!();
    let mut h = TuiTestHarness::new("typing_compose_motion");
    h.append_config("[session]\ntyping_compose_window_ms = 300");
    add_running_session(&h, "TypeMe");

    h.spawn_tui();
    h.wait_for_ready();
    h.wait_for("TypeMe");

    h.send_keys_unfenced("z");
    for i in 0..20 {
        h.send_mouse_move_unfenced(10 + i, 10);
        std::thread::sleep(Duration::from_millis(50));
    }
    h.send_keys("q");
    h.wait_for("Quit Agent of Empires");
    h.assert_screen_not_contains(" > TypeMe ");
}
