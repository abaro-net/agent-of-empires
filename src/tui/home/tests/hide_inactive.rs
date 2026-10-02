//! `y` hides stopped sessions inside groups and shows `shown/total` on their headers.

use super::*;
use crate::session::config::{GroupByMode, SortOrder};
use crate::session::Status;

fn with_status(mut inst: Instance, status: Status) -> Instance {
    inst.status = status;
    inst
}

/// `util`: one idle session and two stopped; `ops`: two stopped; one stopped session in no group.
fn env_with_stopped(manual: bool) -> TestEnv {
    let instances = [
        with_status(instance_in("util-live", "/tmp/util", "util"), Status::Idle),
        with_status(
            instance_in("util-stopped-a", "/tmp/util", "util"),
            Status::Stopped,
        ),
        with_status(
            instance_in("util-stopped-b", "/tmp/util", "util"),
            Status::Stopped,
        ),
        with_status(
            instance_in("ops-stopped-a", "/tmp/ops", "ops"),
            Status::Stopped,
        ),
        with_status(
            instance_in("ops-stopped-b", "/tmp/ops", "ops"),
            Status::Stopped,
        ),
        with_status(
            Instance::new("loose-stopped", "/tmp/loose"),
            Status::Stopped,
        ),
    ];
    seeded_env(test_home(), &instances, manual)
}

fn session_titles(view: &HomeView) -> Vec<String> {
    view.flat_items
        .iter()
        .filter_map(|item| match item {
            Item::Session { id, .. } => view.get_instance(id).map(|i| i.title.clone()),
            Item::Group { .. } => None,
        })
        .collect()
}

fn header_text(view: &HomeView, group: &str) -> String {
    let item = view
        .flat_items
        .iter()
        .find(|item| matches!(item, Item::Group { path, .. } if path == group))
        .unwrap_or_else(|| panic!("no {group} header"));
    rendered_row_text(view, item)
}

fn press_y(env: &mut TestEnv) {
    env.view.handle_key(key(KeyCode::Char('y')), None);
}

#[test]
#[serial]
fn y_hides_stopped_sessions_in_groups_and_counts_them_on_the_header() {
    let mut env = env_with_stopped(true);
    assert_eq!(session_titles(&env.view).len(), 6);
    assert!(header_text(&env.view, "util").contains("util (3)"));

    press_y(&mut env);
    let shown = session_titles(&env.view);
    assert!(shown.contains(&"util-live".to_string()));
    assert!(
        shown.contains(&"loose-stopped".to_string()),
        "a session in no group is never hidden"
    );
    assert!(shown.iter().all(|t| !t.contains("-stopped-")), "{shown:?}");
    assert!(header_text(&env.view, "util").contains("util (1/3)"));
    assert!(
        header_text(&env.view, "ops").contains("ops (0/2)"),
        "a group whose sessions are all hidden keeps its header"
    );

    press_y(&mut env);
    assert_eq!(session_titles(&env.view).len(), 6);
    assert!(header_text(&env.view, "util").contains("util (3)"));
}

/// Project grouping derives its groups from the repo, so hiding follows those groups too.
#[test]
#[serial]
fn project_grouping_hides_by_its_own_groups() {
    let mut env = env_with_stopped(false);
    env.view.group_by = GroupByMode::Project;
    env.view.rebuild_flat_items();
    assert_eq!(session_titles(&env.view).len(), 6);

    press_y(&mut env);
    assert_eq!(session_titles(&env.view), vec!["util-live".to_string()]);
    assert!(header_text(&env.view, "util").contains("util (1/3)"));
    assert!(header_text(&env.view, "loose").contains("loose (0/1)"));
}

/// Attention sort lists sessions without groups, and archived sessions keep their own section,
/// so neither loses a row.
#[test]
#[serial]
fn attention_sort_and_the_archived_section_hide_nothing() {
    let mut archived = with_status(
        instance_in("util-archived", "/tmp/util", "util"),
        Status::Stopped,
    );
    archived.archived_at = Some(chrono::Utc::now());
    let mut env = seeded_env(
        test_home(),
        &[
            with_status(instance_in("util-live", "/tmp/util", "util"), Status::Idle),
            with_status(
                instance_in("util-stopped", "/tmp/util", "util"),
                Status::Stopped,
            ),
            archived,
        ],
        true,
    );
    env.view.archived_section_collapsed = false;
    env.view.rebuild_flat_items();

    press_y(&mut env);
    let shown = session_titles(&env.view);
    assert!(!shown.contains(&"util-stopped".to_string()), "{shown:?}");
    assert!(
        shown.contains(&"util-archived".to_string()),
        "the Archived section is not a group to hide in: {shown:?}"
    );

    env.view.apply_sort_order(SortOrder::Attention);
    let shown = session_titles(&env.view);
    assert!(
        shown.contains(&"util-stopped".to_string()),
        "Attention sort has no groups: {shown:?}"
    );
}

/// The cursor on a session that gets hidden lands on a row that is still there.
#[test]
#[serial]
fn hiding_the_selected_session_moves_the_selection_to_a_shown_row() {
    let mut env = env_with_stopped(true);
    let row = env
        .view
        .flat_items
        .iter()
        .position(|item| matches!(item, Item::Session { id, .. } if env.view.get_instance(id).is_some_and(|i| i.title == "util-stopped-a")))
        .expect("util-stopped-a row");
    env.view.cursor = row;
    env.view.update_selected();

    press_y(&mut env);
    assert_eq!(env.view.selected_session, None);
    assert_eq!(env.view.selected_group.as_deref(), Some("util"));
    assert!(matches!(
        &env.view.flat_items[env.view.cursor],
        Item::Group { path, .. } if path == "util"
    ));
}

/// Rows above the selection can drop out in the same rebuild, as when a reload brings in
/// sessions stopped elsewhere; the selection still lands on its own group's header.
#[test]
#[serial]
fn the_selection_finds_its_own_header_when_rows_above_it_also_drop_out() {
    let mut env = seeded_env(
        test_home(),
        &[
            with_status(
                instance_in("first-live", "/tmp/first", "first"),
                Status::Idle,
            ),
            with_status(
                instance_in("first-other", "/tmp/first", "first"),
                Status::Idle,
            ),
            with_status(
                instance_in("second-live", "/tmp/second", "second"),
                Status::Idle,
            ),
            with_status(
                instance_in("second-other", "/tmp/second", "second"),
                Status::Idle,
            ),
            with_status(
                instance_in("third-live", "/tmp/third", "third"),
                Status::Idle,
            ),
        ],
        true,
    );
    // Alphabetical, so hiding a group's only live session cannot also reorder the headers.
    env.view.apply_sort_order(SortOrder::AZ);
    press_y(&mut env);
    let selected = select_session(&mut env, "second-live");
    let upper_live = env
        .view
        .instances
        .values()
        .find(|i| i.title == "first-live")
        .map(|i| i.id.clone())
        .unwrap();

    for id in [&upper_live, &selected] {
        env.view
            .mutate_instance(id, |inst| inst.status = Status::Stopped);
    }
    env.view.rebuild_flat_items_keeping_cursor();
    assert_eq!(env.view.selected_group.as_deref(), Some("second"));
}

#[test]
#[serial]
fn strict_mode_toggles_on_shift_y() {
    let mut env = env_with_stopped(true);
    env.view.strict_hotkeys = true;
    env.view.handle_key(key(KeyCode::Char('Y')), None);
    assert!(header_text(&env.view, "util").contains("util (1/3)"));
}

fn select_session(env: &mut TestEnv, title: &str) -> String {
    let (row, id) = env
        .view
        .flat_items
        .iter()
        .enumerate()
        .find_map(|(row, item)| match item {
            Item::Session { id, .. }
                if env.view.get_instance(id).is_some_and(|i| i.title == title) =>
            {
                Some((row, id.clone()))
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("no {title} row"));
    env.view.cursor = row;
    env.view.update_selected();
    id
}

/// A session stopped while hiding is on leaves the list at once, and the selection stays with
/// its group instead of jumping to the row that slid up. Starting it again brings it back.
#[test]
#[serial]
fn a_session_crossing_into_or_out_of_stopped_follows_the_toggle_at_once() {
    let mut env = env_with_stopped(true);
    press_y(&mut env);
    let live = select_session(&mut env, "util-live");

    env.view.set_instance_status(&live, Status::Stopped);
    assert!(!session_titles(&env.view).contains(&"util-live".to_string()));
    assert!(header_text(&env.view, "util").contains("util (0/3)"));
    assert_eq!(env.view.selected_group.as_deref(), Some("util"));
    assert_eq!(env.view.selected_session, None);

    env.view.set_instance_status(&live, Status::Idle);
    assert!(session_titles(&env.view).contains(&"util-live".to_string()));
    assert!(header_text(&env.view, "util").contains("util (1/3)"));
}

/// A move renumbers every sibling in the store, hidden ones included, so it is refused while
/// they are hidden.
#[test]
#[serial]
fn custom_moves_wait_until_stopped_sessions_are_shown() {
    let mut env = env_with_stopped(true);
    env.view.apply_sort_order(SortOrder::Custom);
    press_y(&mut env);
    select_session(&mut env, "util-live");
    let before: Vec<_> = env
        .view
        .instances
        .values()
        .map(|i| (i.id.clone(), i.sort_index, i.group_path.clone()))
        .collect();

    env.view.move_row_at_cursor(1).unwrap();
    assert!(env
        .view
        .status_flash_text()
        .is_some_and(|t| t.contains("Show stopped and snoozed sessions")));
    let after: Vec<_> = env
        .view
        .instances
        .values()
        .map(|i| (i.id.clone(), i.sort_index, i.group_path.clone()))
        .collect();
    assert_eq!(before, after);

    let group_row = env
        .view
        .flat_items
        .iter()
        .position(|item| matches!(item, Item::Group { path, .. } if path == "util"))
        .unwrap();
    env.view.cursor = group_row;
    env.view.update_selected();
    env.view.status_flash = None;
    env.view.move_row_at_cursor(1).unwrap();
    assert!(
        !env.view
            .status_flash_text()
            .is_some_and(|t| t.contains("Show stopped and snoozed sessions")),
        "a group header moves among groups, which hiding does not touch"
    );

    press_y(&mut env);
    select_session(&mut env, "util-live");
    env.view.move_row_at_cursor(-1).unwrap();
    let moved: Vec<_> = env
        .view
        .instances
        .values()
        .map(|i| (i.id.clone(), i.sort_index, i.group_path.clone()))
        .collect();
    assert_ne!(
        before, moved,
        "with everything shown the same move goes through"
    );
}

/// A parent group counts its subgroups' sessions, shown and total alike.
#[test]
#[serial]
fn a_parent_header_counts_its_subgroups() {
    let mut env = seeded_env(
        test_home(),
        &[
            with_status(instance_in("util-live", "/tmp/util", "util"), Status::Idle),
            with_status(
                instance_in("sub-live", "/tmp/sub", "util/sub"),
                Status::Idle,
            ),
            with_status(
                instance_in("sub-stopped", "/tmp/sub", "util/sub"),
                Status::Stopped,
            ),
        ],
        true,
    );
    press_y(&mut env);
    assert!(header_text(&env.view, "util").contains("util (2/3)"));
    assert!(header_text(&env.view, "util/sub").contains("sub (1/2)"));
}

/// The all-profiles list can hold the same group path twice; each header counts its own profile.
#[test]
#[serial]
fn each_profile_counts_its_own_copy_of_a_group() {
    let (_temp, _guard) = test_home();
    seed_profile(
        "alpha",
        &[
            with_status(instance_in("a-live", "/tmp/a", "util"), Status::Idle),
            with_status(instance_in("a-stopped", "/tmp/a", "util"), Status::Stopped),
        ],
    );
    seed_profile(
        "beta",
        &[
            with_status(
                instance_in("b-stopped-1", "/tmp/b", "util"),
                Status::Stopped,
            ),
            with_status(
                instance_in("b-stopped-2", "/tmp/b", "util"),
                Status::Stopped,
            ),
            with_status(
                instance_in("b-stopped-3", "/tmp/b", "util"),
                Status::Stopped,
            ),
        ],
    );
    let mut view = test_view(None);
    view.group_by = GroupByMode::Manual;
    view.rebuild_flat_items();
    view.update_selected();
    view.handle_key(key(KeyCode::Char('y')), None);

    let header_for = |profile: &str| {
        let item = view
            .flat_items
            .iter()
            .find(|item| {
                matches!(item, Item::Group { path, profile: p, .. }
                    if path == "util" && p.as_deref() == Some(profile))
            })
            .unwrap_or_else(|| panic!("no util header for {profile}"));
        rendered_row_text(&view, item)
    };
    assert!(
        header_for("alpha").contains("util (1/2)"),
        "{}",
        header_for("alpha")
    );
    assert!(
        header_for("beta").contains("util (0/3)"),
        "{}",
        header_for("beta")
    );
}

#[test]
#[serial]
fn the_toggle_says_what_it_did() {
    let mut env = env_with_stopped(true);
    press_y(&mut env);
    assert!(env
        .view
        .status_flash_text()
        .is_some_and(|t| t.contains("hidden")));
    press_y(&mut env);
    assert!(env
        .view
        .status_flash_text()
        .is_some_and(|t| t.contains("Showing")));
}

/// A hidden structured session the daemon reports working again comes back into its group.
#[test]
#[serial]
fn a_structured_session_lifted_out_of_stopped_reappears() {
    let mut acp = with_status(
        instance_in("acp-stopped", "/tmp/util", "util"),
        Status::Stopped,
    );
    acp.tool = "claude".into();
    acp.view = crate::session::View::Structured;
    let id = acp.id.clone();
    let mut env = seeded_env(
        test_home(),
        &[
            with_status(instance_in("util-live", "/tmp/util", "util"), Status::Idle),
            acp,
        ],
        true,
    );
    press_y(&mut env);
    assert!(!session_titles(&env.view).contains(&"acp-stopped".to_string()));

    env.view
        .apply_daemon_status_update(crate::tui::session_feed::DaemonStatusUpdate {
            id,
            status: Status::Running,
            last_error: None,
            last_accessed_at: None,
            idle_entered_at: None,
            pending_approvals: Vec::new(),
        });
    assert!(session_titles(&env.view).contains(&"acp-stopped".to_string()));
    assert!(header_text(&env.view, "util").contains("util (2)"));
}

fn snoozed(mut inst: Instance) -> Instance {
    inst.snooze(60);
    inst
}

/// A snoozed session sits out of its group like a stopped one; one in no group stays shown.
#[test]
#[serial]
fn y_hides_snoozed_sessions_in_groups_too() {
    let mut env = seeded_env(
        test_home(),
        &[
            with_status(instance_in("util-live", "/tmp/util", "util"), Status::Idle),
            snoozed(with_status(
                instance_in("util-snoozed", "/tmp/util", "util"),
                Status::Idle,
            )),
            snoozed(with_status(
                Instance::new("loose-snoozed", "/tmp/loose"),
                Status::Idle,
            )),
        ],
        true,
    );
    assert!(session_titles(&env.view).contains(&"util-snoozed".to_string()));

    press_y(&mut env);
    let shown = session_titles(&env.view);
    assert!(!shown.contains(&"util-snoozed".to_string()), "{shown:?}");
    assert!(shown.contains(&"loose-snoozed".to_string()), "{shown:?}");
    assert!(header_text(&env.view, "util").contains("util (1/2)"));
}

/// A hidden snoozed session that starts waiting on the user wakes and comes back into its
/// group, and the wake is saved so a reload does not snooze it again.
#[test]
#[serial]
fn a_hidden_snoozed_session_that_starts_waiting_comes_back() {
    let watcher = snoozed(with_status(
        instance_in("util-watcher", "/tmp/util", "util"),
        Status::Running,
    ));
    let id = watcher.id.clone();
    let mut env = seeded_env(
        test_home(),
        &[
            with_status(instance_in("util-live", "/tmp/util", "util"), Status::Idle),
            watcher,
        ],
        true,
    );
    press_y(&mut env);
    assert!(!session_titles(&env.view).contains(&"util-watcher".to_string()));

    env.view
        .apply_status_updates_without_hooks(vec![status_update(
            &id,
            Status::Idle,
            crate::tui::status_poller::IdleIntent::Keep,
        )]);
    assert!(
        !session_titles(&env.view).contains(&"util-watcher".to_string()),
        "a finished turn leaves it snoozed"
    );

    env.view
        .apply_status_updates_without_hooks(vec![status_update(
            &id,
            Status::Waiting,
            crate::tui::status_poller::IdleIntent::Keep,
        )]);
    assert!(session_titles(&env.view).contains(&"util-watcher".to_string()));
    assert!(header_text(&env.view, "util").contains("util (2)"));
    let disk = Storage::new_unwatched("test").unwrap().load().unwrap();
    assert!(!disk.iter().find(|i| i.id == id).unwrap().is_snoozed());
}

/// `h` snoozes a session row in a grouped sort; on the group header it still collapses.
#[test]
#[serial]
fn h_snoozes_a_session_row_and_still_collapses_a_group_header() {
    let mut env = env_with_stopped(true);
    select_session(&mut env, "util-live");
    env.view.handle_key(key(KeyCode::Char('h')), None);
    assert!(
        env.view.snooze_duration_dialog.is_some(),
        "h on a session row opens the snooze picker"
    );
    env.view.snooze_duration_dialog = None;

    let header = env
        .view
        .flat_items
        .iter()
        .position(|item| matches!(item, Item::Group { path, .. } if path == "util"))
        .unwrap();
    env.view.cursor = header;
    env.view.update_selected();
    env.view.handle_key(key(KeyCode::Char('h')), None);
    assert!(env.view.snooze_duration_dialog.is_none());
    assert!(
        matches!(
            &env.view.flat_items[env.view.cursor],
            Item::Group {
                collapsed: true,
                ..
            }
        ),
        "h on a group header collapses it"
    );
}

/// Snoozing the selected grouped session while hiding is on hands the selection to its group,
/// and waking it brings the row back.
#[test]
#[serial]
fn snoozing_the_selected_session_while_hidden_keeps_the_selection_in_its_group() {
    let mut env = env_with_stopped(true);
    press_y(&mut env);
    let id = select_session(&mut env, "util-live");

    env.view.snooze_session_for(&id, 60).unwrap();
    assert!(!session_titles(&env.view).contains(&"util-live".to_string()));
    assert_eq!(env.view.selected_group.as_deref(), Some("util"));

    let header = env
        .view
        .flat_items
        .iter()
        .position(|item| matches!(item, Item::Group { path, .. } if path == "util"))
        .unwrap();
    env.view.cursor = header;
    env.view.update_selected();
    env.view.selected_session = Some(id.clone());
    env.view.toggle_snooze_at_cursor().unwrap();
    assert!(session_titles(&env.view).contains(&"util-live".to_string()));
    assert!(
        matches!(&env.view.flat_items[env.view.cursor], Item::Session { id: at, .. } if *at == id),
        "the cursor follows the woken row"
    );
}

/// The TUI wakes only terminal rows; a structured session's status and unread mark belong to
/// the daemon's ACP path, so a waiting one stays snoozed here.
#[test]
#[serial]
fn a_snoozed_structured_session_is_not_woken_by_the_tui() {
    let mut acp = snoozed(with_status(
        instance_in("util-acp", "/tmp/util", "util"),
        Status::Running,
    ));
    acp.tool = "claude".into();
    acp.view = crate::session::View::Structured;
    let id = acp.id.clone();
    let mut env = seeded_env(test_home(), &[acp], true);

    env.view
        .apply_status_updates_without_hooks(vec![status_update(
            &id,
            Status::Waiting,
            crate::tui::status_poller::IdleIntent::Keep,
        )]);
    let inst = env.view.get_instance(&id).unwrap();
    assert_eq!(inst.status, Status::Waiting, "the update itself applied");
    assert!(inst.is_snoozed());
}

fn waiting(env: &mut TestEnv, id: &str) {
    env.view
        .apply_status_updates_without_hooks(vec![status_update(
            id,
            Status::Waiting,
            crate::tui::status_poller::IdleIntent::Keep,
        )]);
}

fn disk_snooze(id: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    Storage::new_unwatched("test")
        .unwrap()
        .load()
        .unwrap()
        .into_iter()
        .find(|i| i.id == id)
        .unwrap()
        .snoozed_until
}

fn env_with_snoozed_watcher() -> (TestEnv, String) {
    let watcher = snoozed(with_status(
        instance_in("util-watcher", "/tmp/util", "util"),
        Status::Running,
    ));
    let id = watcher.id.clone();
    let mut env = seeded_env(
        test_home(),
        &[
            with_status(instance_in("util-live", "/tmp/util", "util"), Status::Idle),
            watcher,
        ],
        true,
    );
    press_y(&mut env);
    (env, id)
}

/// A snooze set elsewhere after this TUI last loaded the row survives the wake.
#[test]
#[serial]
fn a_wake_leaves_a_snooze_set_elsewhere_alone() {
    let (mut env, id) = env_with_snoozed_watcher();
    let newer =
        env.view.get_instance(&id).unwrap().snoozed_until.unwrap() + chrono::Duration::minutes(30);
    Storage::new_unwatched("test")
        .unwrap()
        .update(|insts, _| {
            insts.iter_mut().find(|i| i.id == id).unwrap().snoozed_until = Some(newer);
            Ok(())
        })
        .unwrap();

    waiting(&mut env, &id);

    assert_eq!(disk_snooze(&id), Some(newer));
    assert!(!session_titles(&env.view).contains(&"util-watcher".to_string()));
}

/// A wake whose write fails stays pending and lands on a later poll, with no second edge into
/// Waiting; until then the session stays hidden, so screen and disk agree.
#[test]
#[serial]
fn a_wake_that_failed_to_persist_lands_on_a_later_poll() {
    let (mut env, id) = env_with_snoozed_watcher();
    let sessions = crate::session::get_profile_dir("test")
        .unwrap()
        .join("sessions.json");
    let parked = sessions.with_extension("json.parked");
    std::fs::rename(&sessions, &parked).unwrap();
    std::fs::create_dir(&sessions).unwrap();

    waiting(&mut env, &id);
    assert!(!session_titles(&env.view).contains(&"util-watcher".to_string()));

    std::fs::remove_dir(&sessions).unwrap();
    std::fs::rename(&parked, &sessions).unwrap();
    env.view.apply_status_updates_without_hooks(Vec::new());

    assert!(session_titles(&env.view).contains(&"util-watcher".to_string()));
    assert_eq!(disk_snooze(&id), None);
}

/// A session menu closes when its session drops out under it, rather than acting on the group
/// header the selection falls back to.
#[test]
#[serial]
fn a_session_menu_closes_when_its_session_is_hidden() {
    let mut env = env_with_stopped(true);
    press_y(&mut env);
    let id = select_session(&mut env, "util-live");
    env.view.list_inner_area = ratatui::layout::Rect::new(1, 1, 28, 20);
    env.view.list_area = ratatui::layout::Rect::new(0, 0, 30, 22);
    assert!(env
        .view
        .handle_right_click(5, env.view.list_inner_area.y + env.view.cursor as u16));
    assert_eq!(env.view.selected_session.as_deref(), Some(id.as_str()));
    assert!(env.view.context_menu.is_some());

    Storage::new_unwatched("test")
        .unwrap()
        .update(|insts, _| {
            insts.iter_mut().find(|i| i.id == id).unwrap().snooze(60);
            Ok(())
        })
        .unwrap();
    env.view.reload_storage_only().unwrap();

    assert_eq!(env.view.selected_group.as_deref(), Some("util"));
    assert!(env.view.context_menu.is_none());
}
