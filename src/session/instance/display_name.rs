//! Keeping the agent's own session name in step with the AoE title.
//!
//! A title reaches the agent at two moments. At launch it rides the command line as the
//! agent's name flag (`launch_command::apply_display_name`). Under [`PushTitleMode::Live`] a
//! rename also reaches a pane that is already running: it is parked on the row, and a worker
//! types it once it holds the instance lifecycle lock that every rename caller is still
//! holding. Everything the send depends on, the pane's name included, is read after that
//! wait: a rename renames the pane, a restart replaces it, and the other poller's worker may
//! have typed the same title already.

use super::*;
use crate::session::config::PushTitleMode;
use crate::tmux::SessionExistence;

/// What to do with a parked title, given what tmux says about the pane.
enum ParkDisposition {
    Send,
    /// tmux could not be queried; the next idle edge tries again.
    Hold,
    /// The pane is gone. The launch flag carries the title at the next start for every title
    /// it accepts, and a title it skips is one the agent would have refused anyway.
    Drop,
}

impl Instance {
    pub(crate) fn push_title_mode(&self) -> PushTitleMode {
        crate::session::config::profile_config::resolve_config_or_warn(&self.effective_profile())
            .session
            .push_title
    }

    /// Everything about typing a rename that does not depend on what the pane is doing now. A
    /// structured session has no pane to type into, and a command override may be a wrapper or
    /// a shell, which would run the text as a command.
    pub(crate) fn accepts_typed_rename(&self) -> bool {
        self.push_title_mode() == PushTitleMode::Live
            && !self.is_structured()
            && !self.has_command_override()
            && crate::agents::get_agent(&self.tool)
                .and_then(|agent| agent.display_name_command())
                .is_some()
    }

    /// The agent key whose manifest describes this pane. An `agent_detect_as` alias wins, but
    /// an empty one is only the absence of an alias, so it falls back to the tool: passed on as
    /// `""` it would match no manifest at all, and nothing would ever be detected.
    fn manifest_tool(&self) -> std::borrow::Cow<'_, str> {
        let alias = self.effective_detect_as();
        if alias.is_empty() {
            std::borrow::Cow::Borrowed(self.tool.as_str())
        } else {
            alias
        }
    }

    /// Ask the pane itself, through the detector the pollers use: the agent's status hook, its
    /// manifest rules and the pane title all count here. A row read from disk cannot answer
    /// this, since it is written after the edge it reports.
    fn pane_reports_idle(&self, pane: &crate::tmux::Session) -> Result<bool> {
        let profile = self.effective_profile();
        let rules_tool =
            crate::tmux::status_rules::detection_tool(&profile, &self.tool, &self.detect_as);
        let manifest_tool = self.manifest_tool();
        let hook = crate::hooks::read_hook_status(&self.id).map(|status| {
            crate::tmux::detect::HookObservation {
                status,
                age: crate::hooks::read_hook_status_age(&self.id),
            }
        });
        let content = pane.capture_pane(50)?;
        let osc_title = crate::tmux::utils::pane_title(pane.name()).unwrap_or_default();
        Ok(detection_proves_idle(crate::tmux::detect_with_rules(
            &profile,
            &rules_tool,
            &manifest_tool,
            &crate::tmux::utils::strip_ansi(&content),
            &osc_title,
            hook,
        )))
    }
}

/// The keystrokes that rename the agent, or `None` when `title` cannot be typed. The command
/// is typed, so a control character would submit it early and leave the rest in the composer.
/// Padding is `tmux::submit_text`'s to add: the argument separator already closes the
/// autocomplete that a leading `/` opens.
fn rename_keystrokes(template: &str, title: &str) -> Option<String> {
    let title: String = title
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let title = title.trim();
    if title.is_empty() || !template.contains("{}") {
        return None;
    }
    Some(template.replace("{}", title))
}

/// Whether the pane can take typed input at all. A tombstone pane left by `remain-on-exit`
/// has nothing listening, and a live pane that has fallen back to a shell would run the
/// rename as a command: an agent that exited leaves a screen its own rules still match, so
/// neither is caught by asking the detector.
fn pane_can_take_input(is_dead: bool, runs_shell: bool) -> bool {
    !is_dead && !runs_shell
}

/// Whether a detection proves the pane is at rest. A screen nothing matched says only that the
/// capture failed or the agent is unknown, and a rule that read the screen without reading a
/// state (a transcript or a model picker is open) says only that the pane is not showing its
/// prompt. Both are the absence of evidence; typing on either lands keystrokes in a pane
/// mid-turn, or an Enter in a modal.
fn detection_proves_idle(detection: Option<crate::tmux::detect::Detection>) -> bool {
    matches!(detection, Some(detection) if detection.status == Some(Status::Idle))
}

fn park_disposition(existence: SessionExistence) -> ParkDisposition {
    match existence {
        SessionExistence::Present => ParkDisposition::Send,
        SessionExistence::Absent => ParkDisposition::Drop,
        SessionExistence::Unknown => ParkDisposition::Hold,
    }
}

/// Whether a parked title is still worth typing. A title the row has moved on from belongs to
/// a rename that has been superseded, and a session that may not be typed into keeps its title
/// through the launch flag alone.
fn should_type_parked_title(inst: &Instance, parked: &str) -> bool {
    parked == inst.title && inst.accepts_typed_rename()
}

/// Follow-up for a rename path that has a worker to outlive it: park the new title and let a
/// detached thread wait for the lock the caller still holds. Reports whether it parked, so a
/// caller holding the row in memory can keep it in step.
pub(crate) fn push_renamed_title(profile: &str, id: &str, title: &str) -> Result<bool> {
    if !park_renamed_title(profile, id, title)? {
        return Ok(false);
    }
    let profile = profile.to_string();
    let id = id.to_string();
    std::thread::spawn(move || log_flush_failure(&profile, &id));
    Ok(true)
}

/// How long a process that is about to exit waits for the send. A peer holding this session's
/// lifecycle lock (a start, a restart, a container build) must not hang a rename that is
/// already persisted; the park then falls to the pollers, the same as a busy pane does.
const BLOCKING_SEND_BUDGET: std::time::Duration = std::time::Duration::from_secs(5);

/// The same follow-up for a process that is about to exit, which a detached thread would not
/// survive: the send runs here, so the caller must already have released the instance
/// lifecycle lock.
pub(crate) fn push_renamed_title_blocking(profile: &str, id: &str, title: &str) -> Result<bool> {
    if !park_renamed_title(profile, id, title)? {
        return Ok(false);
    }
    let worker_profile = profile.to_string();
    let worker_id = id.to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        log_flush_failure(&worker_profile, &worker_id);
        let _ = tx.send(());
    });
    if rx.recv_timeout(BLOCKING_SEND_BUDGET).is_err() {
        tracing::debug!(target: "session.display_name", session = %id, "agent rename left to the status pollers");
    }
    Ok(true)
}

/// Send a title parked while the agent was working, once its turn ends. Detached and
/// self-gating, in the shape of `maybe_spawn_terminal_smart_rename`: the common case is a row
/// with nothing parked.
pub(crate) fn maybe_flush_pending_agent_title(inst: &Instance) {
    if inst.pending_agent_title.is_none() {
        return;
    }
    let profile = inst.source_profile.clone();
    let id = inst.id.clone();
    std::thread::spawn(move || log_flush_failure(&profile, &id));
}

fn log_flush_failure(profile: &str, id: &str) {
    if let Err(error) = flush_pending_agent_title(profile, id) {
        tracing::warn!(target: "session.display_name", session = %id, "agent rename failed: {error}");
    }
}

fn park_renamed_title(profile: &str, id: &str, title: &str) -> Result<bool> {
    // Cheapest gate first: with typing off, no rename should read the whole store.
    if crate::session::config::profile_config::resolve_config_or_warn(
        &crate::session::config::effective_profile(profile),
    )
    .session
    .push_title
        != PushTitleMode::Live
    {
        return Ok(false);
    }
    let storage = crate::session::Storage::open_unwatched(profile)?;
    let Some(inst) = storage.load()?.into_iter().find(|i| i.id == id) else {
        return Ok(false);
    };
    if !inst.accepts_typed_rename() {
        return Ok(false);
    }
    park_pending_agent_title(&storage, id, title)?;
    Ok(true)
}

fn flush_pending_agent_title(profile: &str, id: &str) -> Result<()> {
    let storage = crate::session::Storage::open_unwatched(profile)?;
    let Some(waiting) = storage.load()?.into_iter().find(|i| i.id == id) else {
        return Ok(());
    };
    let Some(early_park) = waiting.pending_agent_title.clone() else {
        return Ok(());
    };
    let _input_lock = match waiting.lock_for_input() {
        Ok(lock) => lock,
        // Deleted, archived or trashed while this worker queued: no pane is left to type into,
        // and the launch flag carries the title if it ever starts again.
        Err(error) if park_is_moot(&error) => {
            take_pending_agent_title(&storage, id, &early_park)?;
            return Ok(());
        }
        Err(error) => return Err(error),
    };

    // Read again under the lock. The wait for it is unbounded, so the row that queued this
    // worker is older than the decision: the title, the agent and the setting may all have
    // moved while it waited.
    let Some(inst) = storage.load()?.into_iter().find(|i| i.id == id) else {
        return Ok(());
    };
    let Some(parked) = inst.pending_agent_title.clone() else {
        return Ok(());
    };
    let keystrokes = crate::agents::get_agent(&inst.tool)
        .and_then(|agent| agent.display_name_command())
        .filter(|_| should_type_parked_title(&inst, &parked))
        .and_then(|template| rename_keystrokes(template, &parked));
    let Some(keystrokes) = keystrokes else {
        take_pending_agent_title(&storage, id, &parked)?;
        return Ok(());
    };

    let pane = inst.tmux_session()?;
    match park_disposition(pane.existence()) {
        ParkDisposition::Drop => {
            take_pending_agent_title(&storage, id, &parked)?;
            return Ok(());
        }
        ParkDisposition::Hold => return Ok(()),
        ParkDisposition::Send => {}
    }
    if !pane_can_take_input(pane.is_pane_dead(), pane.is_pane_running_shell()) {
        return Ok(());
    }
    if !inst.pane_reports_idle(&pane)? {
        return Ok(());
    }
    // Claimed before the send: both pollers watch the same idle edge, and a second send would
    // land in the composer of the agent answering the first.
    if !take_pending_agent_title(&storage, id, &parked)? {
        return Ok(());
    }
    // A failed send is not parked again: the text and the Enter go out as separate tmux
    // commands, so a failure can leave the title already typed, and a retry would type it a
    // second time. The launch flag still carries it at the next start.
    let delay = crate::agents::send_keys_enter_delay(&inst.tool);
    pane.send_keys_with_delay(&keystrokes, delay)
}

/// Whether a failure to lock the row for input means the park has nowhere left to go, as
/// opposed to something worth retrying at the next idle edge.
fn park_is_moot(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<crate::session::SessionGone>()
        .is_some()
        || error
            .downcast_ref::<crate::session::StartBlocked>()
            .is_some()
}

fn park_pending_agent_title(
    storage: &crate::session::Storage,
    id: &str,
    title: &str,
) -> Result<()> {
    storage.update(|instances, _groups| {
        if let Some(row) = instances.iter_mut().find(|i| i.id == id) {
            row.pending_agent_title = Some(title.to_string());
        }
        Ok(())
    })
}

/// Take the parked title if it is still `expected`, reporting whether it was. A rename that
/// landed in the meantime parked its own, and dropping that would lose it.
fn take_pending_agent_title(
    storage: &crate::session::Storage,
    id: &str,
    expected: &str,
) -> Result<bool> {
    storage.update(|instances, _groups| {
        let Some(row) = instances.iter_mut().find(|i| i.id == id) else {
            return Ok(false);
        };
        if row.pending_agent_title.as_deref() != Some(expected) {
            return Ok(false);
        }
        row.pending_agent_title = None;
        Ok(true)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::config::profile_config::{save_profile_config, ProfileConfig};

    const PROFILE: &str = "display-name";

    fn set_mode(mode: &str) {
        let mut config = ProfileConfig::default();
        config.overrides.insert(
            "session".to_string(),
            serde_json::json!({ "push_title": mode }),
        );
        save_profile_config(PROFILE, &config).unwrap();
    }

    fn claude_row(title: &str) -> Instance {
        let mut inst = Instance::new(title, "/tmp/display-name");
        inst.tool = "claude".into();
        inst.source_profile = PROFILE.into();
        inst
    }

    fn seed(inst: &Instance) -> crate::session::Storage {
        let storage = crate::session::Storage::open_unwatched(PROFILE).unwrap();
        storage
            .update(|rows, _| {
                rows.push(inst.clone());
                Ok(())
            })
            .unwrap();
        storage
    }

    fn parked(storage: &crate::session::Storage, id: &str) -> Option<String> {
        storage
            .load()
            .unwrap()
            .into_iter()
            .find(|row| row.id == id)
            .and_then(|row| row.pending_agent_title)
    }

    /// The typed payload is one line of plain text, and nothing a control character could cut
    /// short. Padding is `submit_text`'s job, so there is none here.
    #[test]
    fn rename_keystrokes_are_one_plain_line() {
        assert_eq!(
            rename_keystrokes("/rename {}", "Fix login bug").as_deref(),
            Some("/rename Fix login bug")
        );
        assert_eq!(
            rename_keystrokes("/rename {}", "two\nlines\r\nmore").as_deref(),
            Some("/rename two lines  more"),
            "a newline would submit the command early and leave the rest in the composer"
        );
        assert_eq!(
            rename_keystrokes("/rename {}", "  spaced  ").as_deref(),
            Some("/rename spaced")
        );
        assert_eq!(rename_keystrokes("/rename {}", "   "), None);
        assert_eq!(rename_keystrokes("/rename {}", "\u{7}\u{1b}"), None);
        assert_eq!(
            rename_keystrokes("/rename", "plan"),
            None,
            "a template with nowhere to put the title is refused rather than sent bare"
        );
    }

    /// A session with no `agent_detect_as` alias still has to reach its agent's manifest: the
    /// alias is empty for every default-configured session, and an empty key matches no
    /// manifest, so the detector would never report anything at all.
    #[test]
    #[serial_test::serial]
    fn a_session_without_an_alias_still_names_its_own_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let _app = crate::session::test_support::isolate_app_dir_at(&temp.path().join("app"));
        let inst = claude_row("plan");
        assert!(inst.detect_as.is_empty(), "the fixture has no alias");
        assert_eq!(inst.manifest_tool(), "claude");
        assert!(
            crate::tmux::detect::has_manifest(&inst.manifest_tool()),
            "and that key has a manifest, which is what the detector looks up"
        );

        let mut aliased = claude_row("plan");
        aliased.detect_as = "codex".to_string();
        assert_eq!(aliased.manifest_tool(), "codex", "an alias still wins");
    }

    /// Neither a tombstone pane nor one that has fallen back to a shell can take the rename,
    /// and the detector cannot tell: the screen an exited agent leaves behind still matches
    /// its own idle rules.
    #[test]
    fn a_dead_or_shell_pane_takes_no_input() {
        assert!(pane_can_take_input(false, false));
        assert!(
            !pane_can_take_input(true, false),
            "a tombstone pane has nothing listening"
        );
        assert!(
            !pane_can_take_input(false, true),
            "a shell would run the rename as a command"
        );
        assert!(!pane_can_take_input(true, true));
    }

    /// Only a rule that read a state proves the pane is at rest. The two ways a detection can
    /// fail to say anything are the ways a pane gets typed into by mistake.
    #[test]
    fn only_a_detected_state_proves_the_pane_is_at_rest() {
        let detection = |status, rule| {
            Some(crate::tmux::detect::Detection {
                status,
                visible: true,
                rule,
            })
        };
        assert!(detection_proves_idle(detection(
            Some(Status::Idle),
            "ready_prompt"
        )));
        assert!(
            !detection_proves_idle(None),
            "nothing matched the screen, which is a failed capture or an unknown agent"
        );
        assert!(
            !detection_proves_idle(detection(None, "model_picker_menu")),
            "a rule that reads no state leaves the row's own status standing, so an Enter \
             would answer a modal"
        );
        assert!(!detection_proves_idle(detection(
            Some(Status::Running),
            "spinner"
        )));
        assert!(!detection_proves_idle(detection(
            Some(Status::Waiting),
            "approval_prompt"
        )));
    }

    /// Typing into a running pane is the opt-in half of the setting: at `launch` a rename
    /// waits for the next start, and the sessions with no agent pane to type into are out
    /// whatever the setting says.
    #[test]
    #[serial_test::serial]
    fn only_live_mode_accepts_a_typed_rename() {
        let temp = tempfile::tempdir().unwrap();
        let _app = crate::session::test_support::isolate_app_dir_at(&temp.path().join("app"));
        let inst = claude_row("plan");

        for (mode, expected) in [("off", false), ("launch", false), ("live", true)] {
            set_mode(mode);
            assert_eq!(
                inst.accepts_typed_rename(),
                expected,
                "push_title = {mode} decides live typing"
            );
        }

        set_mode("live");
        let mut structured = claude_row("plan");
        structured.view = View::Structured;
        assert!(
            !structured.accepts_typed_rename(),
            "a structured session has no pane to type into"
        );
        let mut wrapped = claude_row("plan");
        wrapped.command = "ssh -t host claude".into();
        assert!(
            !wrapped.accepts_typed_rename(),
            "a command override may be a wrapper"
        );
        let mut shell = claude_row("plan");
        shell.command = "bash".into();
        assert!(
            !shell.accepts_typed_rename(),
            "a shell is an override too, and would run the rename as a command"
        );
        let mut codex = claude_row("plan");
        codex.tool = "codex".into();
        assert!(
            !codex.accepts_typed_rename(),
            "an agent with no documented rename command is left alone"
        );
    }

    /// A rename is parked for the pane only in `live`; the other modes leave the row clean,
    /// so nothing is waiting to be typed into a session the user never opted in for.
    #[test]
    #[serial_test::serial]
    fn a_rename_parks_only_in_live_mode() {
        let temp = tempfile::tempdir().unwrap();
        let _app = crate::session::test_support::isolate_app_dir_at(&temp.path().join("app"));
        set_mode("launch");
        let inst = claude_row("plan");
        let storage = seed(&inst);

        assert!(!park_renamed_title(PROFILE, &inst.id, "renamed").unwrap());
        assert_eq!(parked(&storage, &inst.id), None);

        set_mode("live");
        assert!(park_renamed_title(PROFILE, &inst.id, "renamed").unwrap());
        assert_eq!(parked(&storage, &inst.id), Some("renamed".to_string()));
    }

    /// The parked title is only typed while it is still the session's own: a rename that
    /// happened while the pane was busy is superseded by the next one, and turning the
    /// setting back down takes the queued rename with it.
    #[test]
    #[serial_test::serial]
    fn a_superseded_or_disallowed_park_is_not_typed() {
        let temp = tempfile::tempdir().unwrap();
        let _app = crate::session::test_support::isolate_app_dir_at(&temp.path().join("app"));
        set_mode("live");
        let inst = claude_row("second");

        assert!(should_type_parked_title(&inst, "second"));
        assert!(
            !should_type_parked_title(&inst, "first"),
            "an older rename is dropped rather than typed after the newer one"
        );
        set_mode("launch");
        assert!(
            !should_type_parked_title(&inst, "second"),
            "a queued rename does not survive the setting going back to launch-only"
        );
    }

    /// Taking the park is the claim that keeps two pollers from both typing it, and it only
    /// takes the title it is about: a rename that landed mid-send parked its own.
    #[test]
    #[serial_test::serial]
    fn taking_the_park_claims_only_the_title_it_sent() {
        let temp = tempfile::tempdir().unwrap();
        let _app = crate::session::test_support::isolate_app_dir_at(&temp.path().join("app"));
        set_mode("live");
        let inst = claude_row("plan");
        let storage = seed(&inst);
        park_pending_agent_title(&storage, &inst.id, "second").unwrap();

        assert!(
            !take_pending_agent_title(&storage, &inst.id, "first").unwrap(),
            "the older send does not claim the newer park"
        );
        assert_eq!(parked(&storage, &inst.id), Some("second".to_string()));

        assert!(take_pending_agent_title(&storage, &inst.id, "second").unwrap());
        assert_eq!(parked(&storage, &inst.id), None);
        assert!(
            !take_pending_agent_title(&storage, &inst.id, "second").unwrap(),
            "a second worker finds nothing to claim, so it cannot type it again"
        );
    }

    /// Every rename path calls this while holding the instance lifecycle lock, and the send
    /// needs that same lock. So the call must return having only parked, and the send must
    /// happen on a worker afterwards: done inline it would hang the TUI on a lock it holds.
    /// The row is archived, which makes the worker's own outcome observable: it retires the
    /// park once it gets the lock.
    #[test]
    #[serial_test::serial]
    fn a_rename_parks_inline_and_leaves_the_send_to_a_worker() {
        let temp = tempfile::tempdir().unwrap();
        let _app = crate::session::test_support::isolate_app_dir_at(&temp.path().join("app"));
        set_mode("live");
        let mut inst = claude_row("renamed");
        inst.archived_at = Some(chrono::Utc::now());
        let storage = seed(&inst);
        let lifecycle = storage.acquire_instance_lifecycle_lock(&inst.id).unwrap();

        let id = inst.id.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(push_renamed_title(PROFILE, &id, "renamed"));
        });
        assert!(
            rx.recv_timeout(std::time::Duration::from_secs(5))
                .expect("the rename returns while its caller still holds the lifecycle lock")
                .unwrap(),
            "live mode parks"
        );
        assert_eq!(
            parked(&storage, &inst.id),
            Some("renamed".to_string()),
            "the worker is still waiting for the lock, so the park is untouched"
        );

        drop(lifecycle);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while parked(&storage, &inst.id).is_some() {
            assert!(
                std::time::Instant::now() < deadline,
                "the worker never got the lock it was waiting for"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// tmux failing to answer is not evidence the pane is gone, so the park waits for the next
    /// idle edge instead of being retired on a query that never landed.
    #[test]
    fn an_unqueryable_tmux_holds_the_park() {
        assert!(matches!(
            park_disposition(SessionExistence::Present),
            ParkDisposition::Send
        ));
        assert!(matches!(
            park_disposition(SessionExistence::Absent),
            ParkDisposition::Drop
        ));
        assert!(matches!(
            park_disposition(SessionExistence::Unknown),
            ParkDisposition::Hold
        ));
    }

    /// A rename parked while the agent was busy, on a session that is then restarted: the
    /// launch line already carries that title as the agent's own name, so the park is retired
    /// at the launch commit rather than typed into the fresh pane after its first turn. A park
    /// the launch did not carry is a later rename, and stands.
    #[test]
    #[serial_test::serial]
    fn a_launch_retires_only_the_park_it_delivered() {
        let temp = tempfile::tempdir().unwrap();
        let _app = crate::session::test_support::isolate_app_dir_at(&temp.path().join("app"));
        set_mode("live");
        let mut inst = claude_row("renamed");
        let storage = seed(&inst);
        park_pending_agent_title(&storage, &inst.id, "renamed").unwrap();

        crate::agents::seed_agent_help_for_test(
            "claude",
            "  -n, --name <name>  Set a display name\n",
        );
        let agent = crate::agents::get_agent("claude");
        let mut cmd = "claude".to_string();
        super::super::launch_command::apply_display_name_for_test(&mut cmd, agent, &mut inst);
        assert!(cmd.contains("--name"), "the launch line carried the title");

        let commit = |inst: &mut Instance| {
            inst.acquire_lifecycle_reservation(
                &storage,
                crate::session::instance::lifecycle::LifecycleOperation::Launch,
                Some(Status::Starting),
            )
            .unwrap();
            inst.status = Status::Running;
            inst.commit_lifecycle_launch(&storage, false).unwrap();
        };
        commit(&mut inst);
        assert_eq!(
            parked(&storage, &inst.id),
            None,
            "the park is retired by the launch that delivered it"
        );

        park_pending_agent_title(&storage, &inst.id, "renamed again").unwrap();
        commit(&mut inst);
        assert_eq!(
            parked(&storage, &inst.id),
            Some("renamed again".to_string()),
            "a rename that landed after the argv was built still has to reach the agent"
        );
    }

    /// A park whose row is no longer startable has nowhere to go, so it is retired instead of
    /// being retried at every idle edge for the life of the row.
    #[test]
    #[serial_test::serial]
    fn an_archived_row_retires_its_park() {
        let temp = tempfile::tempdir().unwrap();
        let _app = crate::session::test_support::isolate_app_dir_at(&temp.path().join("app"));
        set_mode("live");
        let mut inst = claude_row("renamed");
        inst.archived_at = Some(chrono::Utc::now());
        let storage = seed(&inst);
        park_pending_agent_title(&storage, &inst.id, "renamed").unwrap();

        flush_pending_agent_title(PROFILE, &inst.id).unwrap();
        assert_eq!(parked(&storage, &inst.id), None);
    }
}
