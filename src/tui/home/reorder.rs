//! Manual row ordering for the home list: move the cursor's session within its group,
//! or its group among the group's siblings. Only meaningful under [`SortOrder::Custom`],
//! where nothing recomputes the order behind the user's back.

use super::*;
use crate::session::config::SortOrder;

impl HomeView {
    /// Move the cursor's row one slot in `delta` (-1 up, 1 down). A session moves among the
    /// sessions of its own group and profile; a group header moves among the groups sharing
    /// its parent. Returns without touching anything in a computed sort, which would discard
    /// the move on the next rebuild.
    pub(super) fn move_row_at_cursor(&mut self, delta: isize) -> anyhow::Result<()> {
        if self.sort_order != SortOrder::Custom {
            self.flash_status("Press o for the Custom sort to arrange rows by hand");
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
        self.renumber(&order)?;
        self.rebuild_flat_items_keeping_cursor();
        Ok(())
    }

    /// Renumber a whole sibling set: sessions that never moved carry no index, and leaving
    /// them unset would sort them behind the ones that did.
    fn renumber(&mut self, order: &[String]) -> anyhow::Result<()> {
        for (position, sibling) in order.iter().enumerate() {
            let position = position as u32;
            if self.instances.get(sibling).and_then(|i| i.sort_index) == Some(position) {
                continue;
            }
            self.apply_user_action(sibling, |inst| inst.sort_index = Some(position))?;
        }
        Ok(())
    }

    /// Group paths in the order the list shows them, including the ungrouped bucket that
    /// `flatten_tree` puts first. The synthetic Archived and Trash sections are left out:
    /// they are sinks a row reaches by being archived or trashed, never by being moved.
    fn displayed_group_order(&self) -> Vec<String> {
        let mut order: Vec<String> = Vec::new();
        for item in &self.flat_items {
            let path = match item {
                Item::Group { path, .. } => path.clone(),
                Item::Session { id, .. } => match self.get_instance(id) {
                    Some(inst) if !inst.is_archived() && !inst.is_trashed() => {
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
    /// group below, so a held key walks the row through the list.
    fn move_session_across_groups(&mut self, id: &str, delta: isize) -> anyhow::Result<()> {
        let Some(current) = self.instances.get(id).map(|i| i.group_path.clone()) else {
            return Ok(());
        };
        let groups = self.displayed_group_order();
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

        self.apply_user_action(id, |inst| inst.group_path = target.clone())?;

        let mut order: Vec<String> = self
            .custom_siblings(id)
            .into_iter()
            .filter(|s| s != id)
            .collect();
        if delta < 0 {
            order.push(id.to_string());
        } else {
            order.insert(0, id.to_string());
        }
        self.renumber(&order)?;
        self.rebuild_flat_items_keeping_cursor();
        Ok(())
    }

    fn move_group_row(&mut self, group_path: &str, delta: isize) -> anyhow::Result<()> {
        let profile = self
            .active_profile
            .clone()
            .or_else(|| self.storages.keys().next().cloned())
            .unwrap_or_default();
        let Some(tree) = self.group_trees.get_mut(&profile) else {
            return Ok(());
        };
        if !tree.move_group(group_path, delta) {
            return Ok(());
        }
        let groups = tree.get_all_groups();
        if let Some(storage) = self.storages.get(&profile) {
            storage.update(|_instances, disk_groups| {
                // `save()` merges groups by path in place, so it cannot express a permutation;
                // reorder the on-disk rows here and leave their contents to `save()`.
                let mut reordered: Vec<Group> = Vec::with_capacity(disk_groups.len());
                for g in &groups {
                    if let Some(existing) = disk_groups.iter().find(|d| d.path == g.path) {
                        reordered.push(existing.clone());
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
