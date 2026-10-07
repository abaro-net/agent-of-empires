//! `y` hides stopped sessions inside groups and shows `shown/total` on their headers; a second
//! press also hides the groups left with nothing shown, and a third shows everything.

use super::*;
use crate::session::config::{GroupByMode, SortOrder};
use crate::session::Status;
use crate::tui::home::StoppedFilter;

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

/// Presses `y` through the rest of the cycle, back to showing everything.
fn show_all(env: &mut TestEnv) {
    for _ in 0..2 {
        if env.view.stopped_filter == StoppedFilter::Off {
            break;
        }
        press_y(env);
    }
    assert_eq!(env.view.stopped_filter, StoppedFilter::Off);
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

    show_all(&mut env);
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

    press_y(&mut env);
    assert!(
        session_titles(&env.view).contains(&"util-archived".to_string()),
        "the Archived section is not emptied by hiding, so it keeps its header"
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
        .is_some_and(|t| t.contains("Show stopped sessions")));
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
            .is_some_and(|t| t.contains("Show stopped sessions")),
        "a group header moves among groups, which hiding does not touch"
    );

    show_all(&mut env);
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
fn the_toggle_says_which_state_it_entered() {
    let mut env = env_with_stopped(true);
    for expected in [
        "Stopped sessions in groups hidden",
        "Groups with nothing shown hidden",
        "Showing",
    ] {
        press_y(&mut env);
        assert!(
            env.view
                .status_flash_text()
                .is_some_and(|t| t.starts_with(expected)),
            "{expected}: {:?}",
            env.view.status_flash_text()
        );
    }
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

/// A session menu closes when its session stops and drops out under it, rather than acting on the
/// group header the selection falls back to.
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

    env.view.set_instance_status(&id, Status::Stopped);

    assert_eq!(env.view.selected_group.as_deref(), Some("util"));
    assert!(env.view.context_menu.is_none());
}

/// Live-send on a session's terminal ends when the session is stopped elsewhere and hiding takes
/// its row away: the terminal can outlive the agent, and keys must not reach a pane no longer
/// shown.
#[test]
#[serial]
fn live_send_ends_when_its_session_is_hidden() {
    let mut env = env_with_stopped(true);
    press_y(&mut env);
    let id = select_session(&mut env, "util-live");
    live_send_to_terminal(&mut env, &id);

    Storage::new_unwatched("test")
        .unwrap()
        .update(|insts, _| {
            let disk = insts.iter_mut().find(|i| i.id == id).unwrap();
            disk.lifecycle_generation += 1;
            disk.status = Status::Stopped;
            Ok(())
        })
        .unwrap();
    env.view.reload_storage_only().unwrap();

    assert!(!session_titles(&env.view).contains(&"util-live".to_string()));
    assert!(env.view.live_send.is_none(), "live-send ends with its row");
    assert_eq!(env.view.selected_group.as_deref(), Some("util"));
}

fn live_send_to_terminal(env: &mut TestEnv, id: &str) {
    let title = env.view.get_instance(id).unwrap().title.clone();
    let mut state = live_send_state(
        id,
        &title,
        &crate::tmux::TerminalSession::resolve_name(id, &title),
    );
    state.target = crate::tui::home::live_send::LiveSendTarget::Terminal;
    env.view.live_send = Some(state);
}

/// Live-send on a stopped session's terminal ends when a sort change (as the sort picker
/// submits it) moves the list out of Attention and the filter hides the session.
#[test]
#[serial]
fn live_send_ends_when_a_sort_change_hides_its_session() {
    let mut env = env_with_stopped(true);
    env.view.apply_sort_order(SortOrder::Attention);
    press_y(&mut env);
    let id = select_session(&mut env, "util-stopped-a");
    live_send_to_terminal(&mut env, &id);
    env.view.rebuild_flat_items();
    assert!(
        env.view.live_send.is_some(),
        "shown under Attention, so still live"
    );

    env.view.apply_sort_order(SortOrder::AZ);

    assert!(!session_titles(&env.view).contains(&"util-stopped-a".to_string()));
    assert!(env.view.live_send.is_none());
}

/// Live-send on an ungrouped stopped session ends when a grouping change (as the group picker
/// submits it) puts the session in a group the filter hides it from.
#[test]
#[serial]
fn live_send_ends_when_a_grouping_change_hides_its_session() {
    let mut env = env_with_stopped(true);
    press_y(&mut env);
    let id = select_session(&mut env, "loose-stopped");
    live_send_to_terminal(&mut env, &id);
    env.view.rebuild_flat_items();
    assert!(
        env.view.live_send.is_some(),
        "ungrouped, so shown and still live"
    );

    env.view.apply_group_by(GroupByMode::Project);

    assert!(!session_titles(&env.view).contains(&"loose-stopped".to_string()));
    assert!(env.view.live_send.is_none());
}

fn cursor_row_session(view: &HomeView) -> Option<String> {
    match view.flat_items.get(view.cursor)? {
        Item::Session { id, .. } => Some(id.clone()),
        Item::Group { .. } => None,
    }
}

/// A sort or grouping change can start hiding the selected session. The selection moves to
/// that session's own group header, never to whichever session now sits at its old index.
#[test]
#[serial]
fn a_projection_change_that_hides_the_selection_selects_its_own_header() {
    type Change = fn(&mut HomeView);
    let cases: [(&str, &str, Change, Change); 2] = [
        (
            "sort",
            "util-stopped-a",
            |view| view.apply_sort_order(SortOrder::Attention),
            |view| view.apply_sort_order(SortOrder::AZ),
        ),
        (
            "grouping",
            "loose-stopped",
            |_| {},
            |view| view.apply_group_by(GroupByMode::Project),
        ),
    ];
    for (case, title, before, change) in cases {
        let mut env = env_with_stopped(true);
        before(&mut env.view);
        press_y(&mut env);
        let id = select_session(&mut env, title);
        assert_eq!(
            env.view.selected_session.as_deref(),
            Some(id.as_str()),
            "{case}"
        );

        change(&mut env.view);

        assert!(
            !session_titles(&env.view).contains(&title.to_string()),
            "{case}: hidden"
        );
        assert_eq!(
            env.view.selected_session, None,
            "{case}: no other session selected"
        );
        let header = match env.view.group_by {
            GroupByMode::Project => {
                super::super::rows::project_group_key(env.view.get_instance(&id).unwrap())
            }
            _ => env.view.get_instance(&id).unwrap().group_path.clone(),
        };
        assert!(
            matches!(&env.view.flat_items[env.view.cursor], Item::Group { path, .. } if *path == header),
            "{case}: cursor on its own {header} header"
        );
        assert_eq!(
            env.view.selected_group.as_deref(),
            Some(header.as_str()),
            "{case}"
        );
    }
}

/// `w` passes over an unread session the filter hides: selecting it would leave the preview and
/// the next action on a row the list does not show. Shown again, the same session is the target.
#[test]
#[serial]
fn w_skips_an_unread_session_the_filter_hides() {
    let unread_before = crate::session::unread_enabled();
    crate::session::set_unread_enabled(true);
    let mut env = env_with_stopped(true);
    let hidden = select_session(&mut env, "util-stopped-a");
    env.view.mutate_instance(&hidden, |inst| inst.mark_unread());
    press_y(&mut env);
    let live = select_session(&mut env, "util-live");

    env.view.handle_key(key(KeyCode::Char('w')), None);
    assert_eq!(env.view.selected_session.as_deref(), Some(live.as_str()));
    assert_eq!(
        cursor_row_session(&env.view).as_deref(),
        Some(live.as_str())
    );

    env.view.info_dialog = None;
    show_all(&mut env);
    select_session(&mut env, "util-live");
    env.view.handle_key(key(KeyCode::Char('w')), None);
    assert_eq!(
        env.view.selected_session.as_deref(),
        Some(hidden.as_str()),
        "shown, the unread session is what w finds"
    );
    assert_eq!(
        cursor_row_session(&env.view).as_deref(),
        Some(hidden.as_str())
    );
    crate::session::set_unread_enabled(unread_before);
}

/// Restoring a stopped session from Archived or Trash while the filter is on brings it back
/// into a group that hides it. The selection follows it to its group's header, so the cursor
/// row and the session the next key acts on agree.
#[test]
#[serial]
fn restoring_a_session_the_filter_hides_selects_its_own_header() {
    type Shelve = fn(&mut HomeView, &str);
    let cases: [(&str, Shelve); 2] = [
        ("archive", |view, id| {
            view.select_session_by_id(id);
            view.toggle_archive_at_cursor().unwrap();
            assert!(view.get_instance(id).unwrap().is_archived());
        }),
        ("trash", |view, id| {
            view.selected_session = Some(id.to_string());
            view.trash_session_by_id(id);
            assert!(view.get_instance(id).unwrap().is_trashed());
        }),
    ];
    for (case, shelve) in cases {
        let mut env = env_with_stopped(true);
        env.view.archived_section_collapsed = false;
        env.view.trashed_section_collapsed = false;
        let id = select_session(&mut env, "util-stopped-a");
        shelve(&mut env.view, &id);
        press_y(&mut env);
        select_session(&mut env, "util-stopped-a");
        assert_eq!(
            env.view.selected_session.as_deref(),
            Some(id.as_str()),
            "{case}: on the shelf"
        );

        env.view.toggle_archive_at_cursor().unwrap();

        let inst = env.view.get_instance(&id).unwrap();
        assert!(
            !inst.is_archived() && !inst.is_trashed(),
            "{case}: restored"
        );
        assert_eq!(
            inst.status,
            Status::Stopped,
            "{case}: restore keeps it stopped"
        );
        assert!(
            !session_titles(&env.view).contains(&"util-stopped-a".to_string()),
            "{case}: hidden"
        );
        assert_eq!(
            env.view.selected_session, None,
            "{case}: the hidden session is not the target"
        );
        assert!(
            matches!(&env.view.flat_items[env.view.cursor], Item::Group { path, .. } if path == "util"),
            "{case}: cursor on its util header"
        );
        assert_eq!(env.view.selected_group.as_deref(), Some("util"), "{case}");
    }
}

fn has_header(view: &HomeView, group: &str) -> bool {
    view.flat_items
        .iter()
        .any(|item| matches!(item, Item::Group { path, .. } if path == group))
}

#[test]
#[serial]
fn a_second_y_hides_groups_left_with_nothing_shown_and_a_third_shows_all() {
    let mut env = env_with_stopped(true);
    press_y(&mut env);
    assert!(header_text(&env.view, "ops").contains("ops (0/2)"));

    press_y(&mut env);
    assert!(!has_header(&env.view, "ops"), "every ops session is hidden");
    assert!(header_text(&env.view, "util").contains("util (1/3)"));
    let mut shown = session_titles(&env.view);
    shown.sort();
    assert_eq!(shown, vec!["loose-stopped", "util-live"]);

    press_y(&mut env);
    assert_eq!(session_titles(&env.view).len(), 6);
    assert!(header_text(&env.view, "ops").contains("ops (2)"));
    assert!(header_text(&env.view, "util").contains("util (3)"));
}

/// A group stays while anything under it is shown, its subgroups included, and goes with
/// everything nested under it once nothing is. A group that never held a session stays.
#[test]
#[serial]
fn emptied_groups_hide_with_their_subgroups_and_parents_follow_their_children() {
    let mut env = seeded_env(
        test_home(),
        &[
            with_status(
                instance_in("mixed-stopped", "/tmp/m", "mixed"),
                Status::Stopped,
            ),
            with_status(
                instance_in("mixed-live", "/tmp/m", "mixed/live"),
                Status::Idle,
            ),
            with_status(
                instance_in("dead-stopped", "/tmp/d", "dead"),
                Status::Stopped,
            ),
            with_status(
                instance_in("dead-sub-stopped", "/tmp/d", "dead/sub"),
                Status::Stopped,
            ),
            with_status(instance_in("half-live", "/tmp/h", "half"), Status::Idle),
            with_status(
                instance_in("half-gone-stopped", "/tmp/h", "half/gone"),
                Status::Stopped,
            ),
        ],
        true,
    );
    let tree = env.view.group_trees.get_mut("test").unwrap();
    tree.create_group("dead/empty");
    tree.create_group("spare");
    env.view.apply_sort_order(SortOrder::AZ);
    press_y(&mut env);
    press_y(&mut env);

    for (group, shown) in [
        ("mixed", true),
        ("mixed/live", true),
        ("half", true),
        ("half/gone", false),
        ("dead", false),
        ("dead/sub", false),
        ("dead/empty", false),
        ("spare", true),
    ] {
        assert_eq!(has_header(&env.view, group), shown, "{group}");
    }
    assert!(header_text(&env.view, "mixed").contains("mixed (1/2)"));
    assert!(header_text(&env.view, "spare").contains("spare (0)"));
    let mut sessions = session_titles(&env.view);
    sessions.sort();
    assert_eq!(sessions, vec!["half-live", "mixed-live"]);
}

#[test]
#[serial]
fn project_grouping_hides_its_own_emptied_groups() {
    let mut env = env_with_stopped(false);
    env.view.group_by = GroupByMode::Project;
    env.view.rebuild_flat_items();
    press_y(&mut env);
    assert!(has_header(&env.view, "loose"));

    press_y(&mut env);
    assert!(!has_header(&env.view, "loose"));
    assert!(!has_header(&env.view, "ops"));
    assert!(header_text(&env.view, "util").contains("util (1/3)"));
}

/// Roots in AZ order with one ungrouped live session above them: `a` all stopped, `b` live
/// with an all-stopped subgroup `b/q`, `c` all stopped, `d` live, `e` all stopped.
fn env_with_groups_to_empty() -> TestEnv {
    let mut env = seeded_env(
        test_home(),
        &[
            with_status(Instance::new("loose-live", "/tmp/loose"), Status::Idle),
            with_status(instance_in("a-stopped", "/tmp/a", "a"), Status::Stopped),
            with_status(instance_in("b-live", "/tmp/b", "b"), Status::Idle),
            with_status(instance_in("q-stopped", "/tmp/b", "b/q"), Status::Stopped),
            with_status(instance_in("c-stopped", "/tmp/c", "c"), Status::Stopped),
            with_status(instance_in("d-live", "/tmp/d", "d"), Status::Idle),
            with_status(instance_in("e-stopped", "/tmp/e", "e"), Status::Stopped),
        ],
        true,
    );
    env.view.apply_sort_order(SortOrder::AZ);
    env
}

fn select_header(env: &mut TestEnv, group: &str) {
    env.view.cursor = env
        .view
        .flat_items
        .iter()
        .position(|item| matches!(item, Item::Group { path, .. } if path == group))
        .unwrap_or_else(|| panic!("no {group} header"));
    env.view.update_selected();
}

fn assert_on_header(view: &HomeView, group: &str, case: &str) {
    assert_eq!(view.selected_session, None, "{case}: no session selected");
    assert_eq!(view.selected_group.as_deref(), Some(group), "{case}");
    assert!(
        matches!(&view.flat_items[view.cursor], Item::Group { path, .. } if path == group),
        "{case}: cursor on the {group} header"
    );
}

/// A selection whose row the second `y` hides moves to its closest enclosing group still shown,
/// else the next shown header, else the previous one; never to a session.
#[test]
#[serial]
fn a_selection_the_second_y_hides_moves_to_the_nearest_shown_header() {
    let cases = [
        ("a", "b", "the next header"),
        ("b/q", "b", "its parent"),
        ("c", "d", "the next header"),
        ("e", "d", "the previous header, nothing shown after it"),
        ("q-stopped", "b", "its group's parent"),
        ("c-stopped", "d", "the header after its group"),
    ];
    for (target, expected, case) in cases {
        let mut env = env_with_groups_to_empty();
        if target.ends_with("-stopped") {
            select_session(&mut env, target);
            press_y(&mut env);
        } else {
            press_y(&mut env);
            select_header(&mut env, target);
        }

        press_y(&mut env);

        assert!(!has_header(&env.view, target), "{case}: {target} hidden");
        assert_on_header(&env.view, expected, &format!("{target} -> {case}"));
    }
}

/// With emptied groups hidden, a group appears and disappears as its last shown session stops
/// and starts. A menu open on a header that goes closes, and the selection moves to the nearest
/// shown header; with no header left, to the nearest shown row.
#[test]
#[serial]
fn a_group_follows_its_last_shown_session_and_a_menu_on_it_closes() {
    let mut env = env_with_groups_to_empty();
    press_y(&mut env);
    press_y(&mut env);
    env.view.list_inner_area = ratatui::layout::Rect::new(1, 1, 28, 20);
    env.view.list_area = ratatui::layout::Rect::new(0, 0, 30, 22);
    select_header(&mut env, "d");
    assert!(env
        .view
        .handle_right_click(5, env.view.list_inner_area.y + env.view.cursor as u16));
    assert!(env.view.context_menu.is_some());
    let d_live = id_of(&env, "d-live");

    env.view.set_instance_status(&d_live, Status::Stopped);
    assert!(!has_header(&env.view, "d"));
    assert!(env.view.context_menu.is_none());
    assert_on_header(&env.view, "b", "d emptied");

    env.view.set_instance_status(&d_live, Status::Idle);
    assert!(header_text(&env.view, "d").contains("d (1)"));
    assert!(session_titles(&env.view).contains(&"d-live".to_string()));

    env.view.set_instance_status(&d_live, Status::Stopped);
    let b_live = id_of(&env, "b-live");
    env.view.set_instance_status(&b_live, Status::Stopped);
    assert!(!has_header(&env.view, "b"));
    let loose = id_of(&env, "loose-live");
    assert_eq!(env.view.selected_session.as_deref(), Some(loose.as_str()));
    assert_eq!(
        cursor_row_session(&env.view).as_deref(),
        Some(loose.as_str())
    );
}

fn id_of(env: &TestEnv, title: &str) -> String {
    env.view
        .instances
        .values()
        .find(|i| i.title == title)
        .map(|i| i.id.clone())
        .unwrap_or_else(|| panic!("no {title}"))
}

/// A group move swaps with its neighbour in the store, which may be a hidden group, so it waits
/// until every group is shown.
#[test]
#[serial]
fn custom_group_moves_wait_until_every_group_is_shown() {
    let mut env = env_with_groups_to_empty();
    env.view.apply_sort_order(SortOrder::Custom);
    let headers = |view: &HomeView| -> Vec<String> {
        view.flat_items
            .iter()
            .filter_map(|item| match item {
                Item::Group { path, .. } => Some(path.clone()),
                Item::Session { .. } => None,
            })
            .collect()
    };
    press_y(&mut env);
    press_y(&mut env);
    select_header(&mut env, "d");
    let before = headers(&env.view);

    env.view.move_row_at_cursor(-1).unwrap();
    assert!(env
        .view
        .status_flash_text()
        .is_some_and(|t| t.contains("Show all groups")));
    assert_eq!(headers(&env.view), before);

    show_all(&mut env);
    select_header(&mut env, "d");
    let shown = headers(&env.view);
    env.view.move_row_at_cursor(-1).unwrap();
    assert_ne!(headers(&env.view), shown, "with every group shown it moves");
}

/// Restoring a stopped session from Archived into a group whose sessions are all stopped leaves
/// that group hidden, so the selection moves to the nearest shown header, not to a session.
#[test]
#[serial]
fn restoring_into_an_emptied_group_selects_the_nearest_shown_header() {
    let mut env = env_with_groups_to_empty();
    env.view.archived_section_collapsed = false;
    let id = select_session(&mut env, "c-stopped");
    env.view.toggle_archive_at_cursor().unwrap();
    assert!(env.view.get_instance(&id).unwrap().is_archived());
    press_y(&mut env);
    press_y(&mut env);
    select_session(&mut env, "c-stopped");

    env.view.toggle_archive_at_cursor().unwrap();

    assert!(!env.view.get_instance(&id).unwrap().is_archived());
    assert!(!has_header(&env.view, "c"));
    assert_on_header(&env.view, "d", "restored into hidden c");
}

/// In the all-profiles list two profiles can hold the same group path. When one profile's copy
/// is hidden, its selection moves within its own profile, never onto the other profile's copy.
#[test]
#[serial]
fn a_hidden_group_never_lands_on_another_profiles_group_of_the_same_path() {
    let (_temp, _guard) = test_home();
    seed_profile(
        "alpha",
        &[with_status(
            instance_in("a-live", "/tmp/a", "util"),
            Status::Idle,
        )],
    );
    seed_profile(
        "beta",
        &[
            with_status(instance_in("b-live", "/tmp/b", "util"), Status::Idle),
            with_status(instance_in("b-stopped", "/tmp/b", "util"), Status::Stopped),
            with_status(instance_in("z-live", "/tmp/z", "zzz"), Status::Idle),
        ],
    );
    let mut view = test_view(None);
    view.group_by = GroupByMode::Manual;
    view.apply_sort_order(SortOrder::AZ);
    view.handle_key(key(KeyCode::Char('y')), None);
    view.handle_key(key(KeyCode::Char('y')), None);
    view.list_inner_area = ratatui::layout::Rect::new(1, 1, 28, 20);
    view.list_area = ratatui::layout::Rect::new(0, 0, 30, 22);
    view.cursor = view
        .flat_items
        .iter()
        .position(|item| {
            matches!(item, Item::Group { path, profile, .. }
                if path == "util" && profile.as_deref() == Some("beta"))
        })
        .expect("beta's util header");
    view.update_selected();
    assert!(view.handle_right_click(5, view.list_inner_area.y + view.cursor as u16));
    assert!(view.context_menu.is_some());
    let b_live = view
        .instances
        .values()
        .find(|i| i.title == "b-live")
        .map(|i| i.id.clone())
        .unwrap();

    view.set_instance_status(&b_live, Status::Stopped);

    assert!(view.context_menu.is_none());
    assert_eq!(view.selected_group.as_deref(), Some("zzz"));
    assert_eq!(view.selected_group_profile.as_deref(), Some("beta"));
    assert!(matches!(
        &view.flat_items[view.cursor],
        Item::Group { path, profile, .. } if path == "zzz" && profile.as_deref() == Some("beta")
    ));
}

/// When hiding empties the whole list, nothing stays selected and a menu open on the row closes,
/// whether the last shown session stopped under it or `y` hid the selected header.
#[test]
#[serial]
fn a_list_emptied_by_hiding_selects_nothing() {
    let mut env = seeded_env(
        test_home(),
        &[
            with_status(instance_in("g-live", "/tmp/g", "g"), Status::Idle),
            with_status(instance_in("g-stopped", "/tmp/g", "g"), Status::Stopped),
        ],
        true,
    );
    press_y(&mut env);
    press_y(&mut env);
    env.view.list_inner_area = ratatui::layout::Rect::new(1, 1, 28, 20);
    env.view.list_area = ratatui::layout::Rect::new(0, 0, 30, 22);
    let id = select_session(&mut env, "g-live");
    assert!(env
        .view
        .handle_right_click(5, env.view.list_inner_area.y + env.view.cursor as u16));
    assert!(env.view.context_menu.is_some());

    env.view.set_instance_status(&id, Status::Stopped);

    assert!(env.view.flat_items.is_empty());
    assert_eq!(env.view.selected_session, None);
    assert!(env.view.context_menu.is_none());

    show_all(&mut env);
    press_y(&mut env);
    select_header(&mut env, "g");
    assert_eq!(env.view.selected_group.as_deref(), Some("g"));

    press_y(&mut env);

    assert!(env.view.flat_items.is_empty());
    assert_eq!(env.view.selected_group, None);
    assert_eq!(env.view.selected_group_profile, None);
}

/// The Archived section sits after every group but is not a neighbouring group, so a hidden last
/// group selects the shown group before it.
#[test]
#[serial]
fn the_archived_section_is_not_a_neighbouring_group() {
    let mut env = env_with_groups_to_empty();
    env.view.archived_section_collapsed = false;
    let id = select_session(&mut env, "a-stopped");
    env.view.toggle_archive_at_cursor().unwrap();
    assert!(env.view.get_instance(&id).unwrap().is_archived());
    press_y(&mut env);
    select_header(&mut env, "e");

    press_y(&mut env);

    assert!(session_titles(&env.view).contains(&"a-stopped".to_string()));
    assert_on_header(&env.view, "d", "e hidden, Archived after it");
}

/// With nothing hidden, a group moved into another profile that already holds its path is still
/// found by path, so the selection follows it rather than landing on an unrelated row.
#[test]
#[serial]
fn a_group_moved_into_a_profile_with_the_same_path_keeps_the_selection() {
    let (_temp, _guard) = test_home();
    seed_profile(
        "alpha",
        &[
            instance_in("a-one", "/tmp/a1", "aaa"),
            instance_in("a-two", "/tmp/a2", "work"),
        ],
    );
    seed_profile(
        "beta",
        &[
            instance_in("b-one", "/tmp/b1", "bbb"),
            instance_in("b-two", "/tmp/b2", "work"),
        ],
    );
    let mut view = test_view(None);
    view.group_by = GroupByMode::Manual;
    view.rebuild_flat_items();
    view.cursor = view
        .flat_items
        .iter()
        .position(|item| {
            matches!(item, Item::Group { path, profile, .. }
                if path == "work" && profile.as_deref() == Some("alpha"))
        })
        .expect("alpha's work header");
    view.update_selected();
    view.group_rename_context = Some(crate::tui::home::GroupRenameContext {
        old_path: "work".to_string(),
        old_profile: "alpha".to_string(),
    });

    view.rename_selected_group(None, Some("beta")).unwrap();

    assert_eq!(view.selected_session, None);
    assert_eq!(view.selected_group.as_deref(), Some("work"));
    assert_eq!(view.selected_group_profile.as_deref(), Some("beta"));
}

/// A menu opened on an empty sidebar has no row to lose, so a reload leaves it open, also when
/// the selection still names the session just deleted.
#[test]
#[serial]
fn a_menu_on_an_empty_sidebar_survives_a_reload() {
    let mut env = seeded_env(test_home(), &[], true);
    assert!(env.view.flat_items.is_empty());
    env.view.context_menu = Some(crate::tui::dialogs::ContextMenuDialog::for_empty_sidebar((
        1, 1,
    )));

    env.view.reload().unwrap();
    assert!(env.view.context_menu.is_some());

    env.view.selected_session = Some("deleted-session".to_string());
    env.view.rebuild_flat_items_keeping_cursor();
    assert!(env.view.context_menu.is_some());
}
