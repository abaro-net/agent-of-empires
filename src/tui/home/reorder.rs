//! Manual row ordering for the home list: move the cursor's session within its group, or its
//! group among the group's siblings. Only meaningful under [`SortOrder::Custom`] and
//! [`GroupByMode::Manual`], where the rows carry the user's own grouping and nothing
//! recomputes their order behind them.

use super::*;
use crate::session::config::{GroupByMode, SortOrder};

impl HomeView {
    /// Move the cursor's row one slot in `delta` (-1 up, 1 down). A session moves among the
    /// sessions of its own group and profile; a group header moves among the groups sharing
    /// its parent. Refuses where a move would be discarded or would write a membership the
    /// list is not showing.
    pub(super) fn move_row_at_cursor(&mut self, delta: isize) -> anyhow::Result<()> {
        if self.sort_order != SortOrder::Custom {
            self.flash_status("Press o for the Custom sort to arrange rows by hand");
            return Ok(());
        }
        if self.group_by != GroupByMode::Manual {
            // Project and Org headers are derived from repo paths, and their rows carry a
            // rewritten `group_path` for display. Moving one would save a manual membership
            // the user cannot see, so the mutation is refused rather than guessed at.
            self.flash_status("Rows are arranged by hand in Manual grouping only (g)");
            return Ok(());
        }
        if let Some(group_path) = self.selected_group.clone() {
            self.move_group_row(&group_path, delta)?;
        } else if let Some(id) = self.selected_session.clone() {
            self.move_session_row(&id, delta)?;
        }
        Ok(())
    }

    /// Sessions that share `id`'s group and profile, in the order the list shows them.
    fn custom_siblings(&self, id: &str) -> Vec<String> {
        let Some(anchor) = self.instances.get(id) else {
            return Vec::new();
        };
        let mut siblings: Vec<&Instance> = self
            .instances
            .values()
            .filter(|i| {
                i.group_path == anchor.group_path
                    && i.source_profile == anchor.source_profile
                    && !i.is_archived()
                    && !i.is_trashed()
            })
            .collect();
        siblings.sort_by_key(|i| {
            (
                i.sort_index.unwrap_or(u32::MAX),
                std::cmp::Reverse(i.created_at),
            )
        });
        siblings.iter().map(|i| i.id.clone()).collect()
    }

    fn move_session_row(&mut self, id: &str, delta: isize) -> anyhow::Result<()> {
        let mut order = self.custom_siblings(id);
        let Some(from) = order.iter().position(|s| s == id) else {
            return Ok(());
        };
        let Some(to) = from.checked_add_signed(delta).filter(|t| *t < order.len()) else {
            // Off the end of its own group: carry on into the neighbouring one.
            return self.move_session_across_groups(id, delta);
        };
        order.swap(from, to);
        self.renumber(&order, None)?;
        self.rebuild_flat_items_keeping_cursor();
        Ok(())
    }

    /// Write a sibling set's positions in one locked update, optionally moving `regroup`'s
    /// session into another group in the same transaction. A per-row write would leave a
    /// half-applied order behind when one of them failed.
    fn renumber(
        &mut self,
        order: &[String],
        regroup: Option<(&str, String)>,
    ) -> anyhow::Result<()> {
        let positions: HashMap<String, u32> = order
            .iter()
            .enumerate()
            .map(|(position, id)| (id.clone(), position as u32))
            .collect();
        let moved = regroup.map(|(id, group)| (id.to_string(), group));
        self.bulk_apply_user_action(order, move |inst| {
            if let Some(position) = positions.get(&inst.id) {
                inst.sort_index = Some(*position);
            }
            if let Some((id, group)) = &moved {
                if inst.id == *id {
                    inst.group_path = group.clone();
                }
            }
        })
    }

    /// Group paths in the order the list shows them, restricted to `profile`, including the
    /// ungrouped bucket that `flatten_tree` puts first. The synthetic Archived and Trash
    /// sections are left out: they are sinks a row reaches by being archived or trashed,
    /// never by being moved.
    fn displayed_group_order(&self, profile: Option<&str>) -> Vec<String> {
        let mut order: Vec<String> = Vec::new();
        for item in &self.flat_items {
            let path = match item {
                Item::Group {
                    path,
                    profile: row_profile,
                    ..
                } => {
                    if profile.is_some() && row_profile.as_deref() != profile {
                        continue;
                    }
                    path.clone()
                }
                Item::Session { id, .. } => match self.get_instance(id) {
                    Some(inst)
                        if !inst.is_archived()
                            && !inst.is_trashed()
                            && profile.is_none_or(|p| inst.source_profile == p) =>
                    {
                        inst.group_path.clone()
                    }
                    _ => continue,
                },
            };
            if crate::session::is_within_archived_section(&path)
                || crate::session::is_within_trash_section(&path)
            {
                continue;
            }
            if !order.contains(&path) {
                order.push(path);
            }
        }
        order
    }

    /// Move a session out of its group into the neighbour in `delta`'s direction, landing
    /// against the boundary it crossed: at the bottom of the group above, the top of the
    /// group below, so a held key walks the row through the list. The neighbour is looked up
    /// inside the row's own profile, so a move never lands it in another profile's group.
    fn move_session_across_groups(&mut self, id: &str, delta: isize) -> anyhow::Result<()> {
        let Some((current, profile)) = self
            .instances
            .get(id)
            .map(|i| (i.group_path.clone(), i.source_profile.clone()))
        else {
            return Ok(());
        };
        let groups = self.displayed_group_order(Some(&profile));
        let Some(at) = groups.iter().position(|g| *g == current) else {
            return Ok(());
        };
        let Some(target) = at
            .checked_add_signed(delta)
            .and_then(|idx| groups.get(idx))
            .cloned()
        else {
            return Ok(());
        };

        let mut order: Vec<String> = self
            .instances
            .values()
            .filter(|i| {
                i.group_path == target
                    && i.source_profile == profile
                    && i.id != id
                    && !i.is_archived()
                    && !i.is_trashed()
            })
            .collect::<Vec<_>>()
            .iter()
            .map(|i| i.id.clone())
            .collect();
        order.sort_by_key(|sibling| {
            self.instances
                .get(sibling)
                .map(|i| {
                    (
                        i.sort_index.unwrap_or(u32::MAX),
                        std::cmp::Reverse(i.created_at),
                    )
                })
                .unwrap_or((u32::MAX, std::cmp::Reverse(chrono::Utc::now())))
        });
        if delta < 0 {
            order.push(id.to_string());
        } else {
            order.insert(0, id.to_string());
        }
        self.renumber(&order, Some((id, target)))?;
        self.rebuild_flat_items_keeping_cursor();
        Ok(())
    }

    fn move_group_row(&mut self, group_path: &str, delta: isize) -> anyhow::Result<()> {
        // The header under the cursor carries its own profile; `active_profile` is only the
        // fallback, and picking any storage key would reorder a different profile's groups.
        let Some(profile) = self
            .selected_group_profile
            .clone()
            .or_else(|| self.active_profile.clone())
        else {
            return Ok(());
        };
        let Some(tree) = self.group_trees.get_mut(&profile) else {
            return Ok(());
        };
        if !tree.move_group(group_path, delta) {
            return Ok(());
        }
        let groups = tree.get_all_groups();
        if let Some(storage) = self.storages.get(&profile) {
            storage.update(|_instances, disk_groups| {
                // `save()` merges groups by path in place, so it cannot express a permutation.
                // A group the tree synthesized from a session's path has no disk row yet and
                // must still be written, or its position is lost on the next load.
                let mut reordered: Vec<Group> = Vec::with_capacity(groups.len());
                for g in &groups {
                    match disk_groups.iter().find(|d| d.path == g.path) {
                        Some(existing) => reordered.push(existing.clone()),
                        None => reordered.push(g.clone()),
                    }
                }
                for d in disk_groups.iter() {
                    if !reordered.iter().any(|r| r.path == d.path) {
                        reordered.push(d.clone());
                    }
                }
                *disk_groups = reordered;
                Ok(())
            })?;
        }
        self.rebuild_flat_items_keeping_cursor();
        Ok(())
    }
}
