//! Atomic ownership changes. Terminal states and runtimes never leave their registries.
use std::collections::{BTreeMap, HashSet};

use crate::layout::PaneId;
use crate::workspace::{public_pane_id_for_number, public_tab_id_for_number, Workspace};

use super::AppState;

pub(crate) enum TabTransferTarget {
    Workspace(usize),
    NewWorkspace {
        label: Option<String>,
        initial_cwd: Option<std::path::PathBuf>,
    },
}

pub(crate) struct TabTransferCommit {
    pub changed: bool,
    pub workspace_index: usize,
    pub tab_index: usize,
    pub previous_workspace_id: String,
    pub previous_tab_id: String,
    pub pane_id_map: BTreeMap<String, String>,
    pub pane_ids: Vec<PaneId>,
    pub created_workspace: bool,
    pub removed_workspace: bool,
}

impl AppState {
    /// Every fallible lookup happens before ownership is taken. The commit has no
    /// PTY operations, filesystem work, callbacks, or observable intermediate state.
    pub(crate) fn transfer_tab(
        &mut self,
        source_workspace: usize,
        source_tab: usize,
        destination: TabTransferTarget,
        focus: bool,
    ) -> Result<TabTransferCommit, &'static str> {
        let source = self
            .workspaces
            .get(source_workspace)
            .ok_or("source workspace not found")?;
        let tab = source.tabs.get(source_tab).ok_or("source tab not found")?;
        let previous_workspace_id = source.id.clone();
        let previous_tab_id = public_tab_id_for_number(&source.id, tab.number);
        let pane_ids = tab.layout.pane_ids();
        if pane_ids.is_empty()
            || pane_ids.len() != tab.panes.len()
            || pane_ids.iter().copied().collect::<HashSet<_>>().len() != pane_ids.len()
            || !tab.panes.contains_key(&tab.layout.focused())
            || pane_ids.iter().any(|id| {
                !tab.panes.contains_key(id)
                    || !source.public_pane_numbers.contains_key(id)
                    || tab
                        .panes
                        .get(id)
                        .is_none_or(|pane| !self.terminals.contains_key(&pane.attached_terminal_id))
            })
        {
            return Err("source tab identity is incomplete");
        }
        if let TabTransferTarget::Workspace(index) = &destination {
            if self.workspaces.get(*index).is_none() {
                return Err("destination workspace not found");
            }
            if *index == source_workspace {
                return Ok(TabTransferCommit {
                    changed: false,
                    workspace_index: source_workspace,
                    tab_index: source_tab,
                    previous_workspace_id,
                    previous_tab_id,
                    pane_id_map: BTreeMap::new(),
                    pane_ids,
                    created_workspace: false,
                    removed_workspace: false,
                });
            }
            let target = &self.workspaces[*index];
            if target
                .tabs
                .iter()
                .any(|tab| tab.panes.keys().any(|id| pane_ids.contains(id)))
            {
                return Err("destination already owns a source pane identity");
            }
            if target.next_public_tab_number
                <= target.tabs.iter().map(|tab| tab.number).max().unwrap_or(0)
                || target.next_public_pane_number
                    <= target
                        .public_pane_numbers
                        .values()
                        .copied()
                        .max()
                        .unwrap_or(0)
            {
                return Err("destination public identity allocation is invalid");
            }
            if target.next_public_tab_number.checked_add(1).is_none()
                || target
                    .next_public_pane_number
                    .checked_add(pane_ids.len())
                    .is_none()
            {
                return Err("destination public identity numbers exhausted");
            }
        }
        let previous_panes: Vec<_> = pane_ids
            .iter()
            .map(|id| {
                (
                    *id,
                    public_pane_id_for_number(&source.id, source.public_pane_numbers[id]),
                )
            })
            .collect();
        let focused = tab.layout.focused();
        let initial_cwd = tab
            .panes
            .get(&focused)
            .and_then(|pane| self.terminals.get(&pane.attached_terminal_id))
            .map(|terminal| terminal.cwd.clone())
            .unwrap_or_else(|| source.identity_cwd.clone());
        let old_active_workspace = self
            .active
            .and_then(|i| self.workspaces.get(i))
            .map(|ws| ws.id.clone());
        let old_selected_workspace = self.workspaces.get(self.selected).map(|ws| ws.id.clone());
        let previous_active_focus = self.current_pane_focus_target();
        let active_pane = previous_active_focus.as_ref().map(|target| target.pane_id);
        let follow = focus || active_pane.is_some_and(|id| pane_ids.contains(&id));
        let target_id = match &destination {
            TabTransferTarget::Workspace(index) => Some(self.workspaces[*index].id.clone()),
            TabTransferTarget::NewWorkspace { .. } => None,
        };
        let created_workspace = target_id.is_none();
        let tab = self.workspaces[source_workspace].take_tab_for_transfer(source_tab);
        let removed_workspace = self.workspaces[source_workspace].tabs.is_empty();
        if removed_workspace {
            // A transfer removes only the empty container, never its worktree group.
            self.workspaces.remove(source_workspace);
        }
        let (workspace_index, tab_index) = match destination {
            TabTransferTarget::Workspace(_) => {
                // Validated stable identity survives any source vector removal.
                let index = self
                    .workspaces
                    .iter()
                    .position(|ws| Some(&ws.id) == target_id.as_ref())
                    .expect("validated destination survives transfer");
                let tab_index = self.workspaces[index].append_existing_tab(tab);
                (index, tab_index)
            }
            TabTransferTarget::NewWorkspace {
                label,
                initial_cwd: live_cwd,
            } => {
                self.workspaces.push(Workspace::from_existing_tab(
                    label,
                    live_cwd.unwrap_or(initial_cwd),
                    tab,
                ));
                (self.workspaces.len() - 1, 0)
            }
        };
        let destination = &self.workspaces[workspace_index];
        let destination_id = destination.id.clone();
        let pane_id_map = previous_panes
            .into_iter()
            .map(|(id, previous)| {
                self.public_pane_id_aliases.insert(previous.clone(), id);
                (
                    previous,
                    public_pane_id_for_number(
                        &destination.id,
                        destination.public_pane_numbers[&id],
                    ),
                )
            })
            .collect();
        let moved: HashSet<_> = pane_ids.iter().copied().collect();
        if let Some(previous) = &mut self.previous_pane_focus {
            if moved.contains(&previous.pane_id) {
                previous.workspace_id = destination_id.clone();
            }
        }
        if let Some(target) = self.toast.as_mut().and_then(|toast| toast.target.as_mut()) {
            if moved.contains(&target.pane_id) {
                target.workspace_id = destination_id.clone();
            }
        }
        for (id, pending) in &mut self.pending_agent_notifications {
            if moved.contains(id) {
                pending.workspace_id = destination_id.clone();
            }
        }
        self.active = old_active_workspace
            .as_ref()
            .and_then(|id| self.workspaces.iter().position(|ws| &ws.id == id))
            .or(Some(source_workspace.min(self.workspaces.len() - 1)));
        self.selected = old_selected_workspace
            .as_ref()
            .and_then(|id| self.workspaces.iter().position(|ws| &ws.id == id))
            .unwrap_or_else(|| source_workspace.min(self.workspaces.len() - 1));
        if follow {
            self.active = Some(workspace_index);
            self.selected = workspace_index;
            self.workspaces[workspace_index].active_tab = tab_index;
            if active_pane != Some(focused) {
                self.previous_pane_focus = previous_active_focus;
            }
            if focus {
                self.mode = super::Mode::Terminal;
            }
        }
        // Geometry belongs to the previous projection and is refreshed authoritatively.
        self.view.pane_infos.clear();
        self.refresh_pane_relocation_aliases();
        self.mark_session_dirty();
        Ok(TabTransferCommit {
            changed: true,
            workspace_index,
            tab_index,
            previous_workspace_id,
            previous_tab_id,
            pane_id_map,
            pane_ids,
            created_workspace,
            removed_workspace,
        })
    }

    /// This cache is rebuilt only when aliases/ownership change, never per frame/client.
    pub(crate) fn refresh_pane_relocation_aliases(&mut self) {
        let current: std::collections::HashMap<_, _> = self
            .workspaces
            .iter()
            .flat_map(|ws| {
                ws.public_pane_numbers
                    .iter()
                    .map(move |(id, number)| (*id, public_pane_id_for_number(&ws.id, *number)))
            })
            .collect();
        let aliases = self
            .public_pane_id_aliases
            .iter()
            .filter_map(|(old, raw)| {
                let new = current.get(raw)?;
                (old != new).then(|| (old.clone(), new.clone()))
            })
            .collect();
        if self.pane_relocation_aliases != aliases {
            self.pane_relocation_aliases = aliases;
            self.pane_relocation_revision = self.pane_relocation_revision.saturating_add(1);
        }
    }

    pub(crate) fn restore_public_pane_aliases(&mut self, aliases: &BTreeMap<String, String>) {
        let current: std::collections::HashMap<_, _> = self
            .workspaces
            .iter()
            .flat_map(|ws| {
                ws.public_pane_numbers
                    .iter()
                    .map(move |(id, number)| (public_pane_id_for_number(&ws.id, *number), *id))
            })
            .collect();
        for (old, new) in aliases {
            if let Some(id) = current.get(new) {
                self.public_pane_id_aliases.insert(old.clone(), *id);
            }
        }
        crate::workspace::reserve_workspace_id_names(
            self.workspaces.iter().map(|ws| ws.id.as_str()).chain(
                aliases
                    .keys()
                    .filter_map(|id| id.split_once(':').map(|(ws, _)| ws)),
            ),
        );
        self.refresh_pane_relocation_aliases();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::{Direction, Rect};

    #[test]
    fn transfer_preserves_adversarial_identity_layout_zoom_terminal_and_focus() {
        for source_first in [true, false] {
            let mut state = AppState::test_with_adversarial_identity_state();
            let source = state.workspaces.remove(0);
            let destination = Workspace::test_adversarial_identity_state();
            state.workspaces = if source_first {
                vec![source, destination]
            } else {
                vec![destination, source]
            };
            let source = usize::from(!source_first);
            let destination = usize::from(source_first);
            let tab_index = state.workspaces[source]
                .tabs
                .iter()
                .position(|tab| tab.panes.len() > 1)
                .unwrap();
            state.workspaces[source].active_tab = tab_index;
            state.workspaces[source].tabs[tab_index].custom_name = Some("live agent".into());
            state.workspaces[source].tabs[tab_index].zoomed = true;
            state.active = Some(source);
            state.selected = source;
            state.ensure_test_terminals();
            let tab = &state.workspaces[source].tabs[tab_index];
            let root = tab.root_pane;
            let pane_ids = tab.layout.pane_ids();
            let focused = tab.layout.focused();
            let shape = tab
                .layout
                .panes(Rect::new(0, 0, 100, 40))
                .into_iter()
                .map(|pane| (pane.id, pane.rect, pane.is_focused))
                .collect::<Vec<_>>();
            let terminals: HashSet<_> = state.terminals.keys().cloned().collect();
            state.workspaces[source].todos.add("source task").unwrap();
            state.workspaces[destination]
                .todos
                .add("destination task")
                .unwrap();
            let destination_tab_count = state.workspaces[destination].tabs.len();
            state.assert_invariants_for_test();
            let commit = state
                .transfer_tab(
                    source,
                    tab_index,
                    TabTransferTarget::Workspace(destination),
                    true,
                )
                .unwrap();
            assert_eq!(state.workspaces[source].todos.items[0].label, "source task");
            assert_eq!(
                state.workspaces[commit.workspace_index].todos.items[0].label,
                "destination task"
            );
            assert!(commit.changed);
            assert_eq!(commit.tab_index, destination_tab_count);
            let moved = &state.workspaces[commit.workspace_index].tabs[commit.tab_index];
            assert_eq!(moved.root_pane, root);
            assert_eq!(moved.layout.pane_ids(), pane_ids);
            assert_eq!(moved.layout.focused(), focused);
            assert_eq!(
                moved
                    .layout
                    .panes(Rect::new(0, 0, 100, 40))
                    .into_iter()
                    .map(|pane| (pane.id, pane.rect, pane.is_focused))
                    .collect::<Vec<_>>(),
                shape
            );
            assert_eq!(moved.custom_name.as_deref(), Some("live agent"));
            assert!(moved.zoomed);
            assert_eq!(
                state.terminals.keys().cloned().collect::<HashSet<_>>(),
                terminals
            );
            assert_eq!(state.active, Some(commit.workspace_index));
            for (old, new) in &commit.pane_id_map {
                assert_ne!(old, new);
                assert_eq!(state.pane_relocation_aliases.get(old), Some(new));
            }
            state.assert_invariants_for_test();
        }
    }

    #[test]
    fn transfer_repeatedly_normalizes_all_retired_aliases_and_removes_only_empty_source() {
        let mut state = AppState::test_new();
        let mut source = Workspace::test_new("source");
        source.test_split(Direction::Vertical);
        let root = source.tabs[0].root_pane;
        state.workspaces = vec![
            source,
            Workspace::test_new("destination"),
            Workspace::test_new("unaffected"),
        ];
        state.active = Some(2);
        state.selected = 2;
        state.ensure_test_terminals();
        let original = public_pane_id_for_number(
            &state.workspaces[0].id,
            state.workspaces[0].public_pane_numbers[&root],
        );
        let first = state
            .transfer_tab(0, 0, TabTransferTarget::Workspace(1), false)
            .unwrap();
        assert!(first.removed_workspace);
        assert_eq!(state.workspaces.len(), 2);
        assert_eq!(state.active, Some(1));
        assert_eq!(
            state.workspaces[1].custom_name.as_deref(),
            Some("unaffected")
        );
        let first_alias = first.pane_id_map[&original].clone();
        let second = state
            .transfer_tab(
                first.workspace_index,
                first.tab_index,
                TabTransferTarget::NewWorkspace {
                    label: Some("third".into()),
                    initial_cwd: None,
                },
                false,
            )
            .unwrap();
        assert!(second.created_workspace);
        assert!(!second.removed_workspace);
        assert_eq!(
            state.pane_relocation_aliases[&original],
            second.pane_id_map[&first_alias]
        );
        assert_eq!(state.public_pane_id_aliases[&original], root);
        assert_eq!(state.public_pane_id_aliases[&first_alias], root);
        assert_eq!(
            state.workspaces[second.workspace_index].worktree_space,
            None
        );
        state.assert_invariants_for_test();
    }

    #[test]
    fn new_destination_uses_focused_terminal_cwd_and_creates_no_terminal() {
        let mut state = AppState::test_new();
        let mut source = Workspace::test_new("source");
        let focused = source.test_split(Direction::Vertical);
        source.tabs[0].layout.focus_pane(focused);
        state.workspaces = vec![source];
        state.active = Some(0);
        state.ensure_test_terminals();
        let terminal = state.workspaces[0].terminal_id(focused).unwrap().clone();
        let cwd = std::path::PathBuf::from("/existing/live/process/directory");
        state.terminals.get_mut(&terminal).unwrap().cwd = cwd.clone();
        let count = state.terminals.len();
        let commit = state
            .transfer_tab(
                0,
                0,
                TabTransferTarget::NewWorkspace {
                    label: None,
                    initial_cwd: None,
                },
                true,
            )
            .unwrap();
        assert!(commit.removed_workspace && commit.created_workspace);
        assert_eq!(state.terminals.len(), count);
        assert_eq!(state.workspaces[0].identity_cwd, cwd);
        assert_eq!(state.workspaces[0].focused_pane_id(), Some(focused));
        state.assert_invariants_for_test();
    }

    #[test]
    fn rejected_and_same_workspace_transfers_leave_state_unchanged() {
        let mut state = AppState::test_with_adversarial_identity_state();
        let before_ids: Vec<_> = state.workspaces[0]
            .tabs
            .iter()
            .map(|tab| tab.root_pane)
            .collect();
        let previous_revision = state.pane_relocation_revision;
        assert!(state
            .transfer_tab(0, 0, TabTransferTarget::Workspace(9), true)
            .is_err());
        assert!(state
            .transfer_tab(
                0,
                usize::MAX,
                TabTransferTarget::NewWorkspace {
                    label: None,
                    initial_cwd: None
                },
                true
            )
            .is_err());
        let commit = state
            .transfer_tab(0, 0, TabTransferTarget::Workspace(0), true)
            .unwrap();
        assert!(!commit.changed);
        assert_eq!(
            state.workspaces[0]
                .tabs
                .iter()
                .map(|tab| tab.root_pane)
                .collect::<Vec<_>>(),
            before_ids
        );
        assert_eq!(state.pane_relocation_revision, previous_revision);
        state.assert_invariants_for_test();
    }

    #[test]
    fn restored_alias_prefix_reserves_workspace_ids_after_highest_source_disappears() {
        let mut state = AppState::test_with_adversarial_identity_state();
        let ws = &state.workspaces[0];
        let pane = ws.tabs[0].root_pane;
        let current = public_pane_id_for_number(&ws.id, ws.public_pane_numbers[&pane]);
        let retired_number = 64;
        let retired_ws = format!(
            "w{}",
            crate::workspace::encode_public_number(retired_number)
        );
        let old = format!("{retired_ws}:p1");
        let aliases = BTreeMap::from([(old.clone(), current)]);
        state.restore_public_pane_aliases(&aliases);
        assert_eq!(state.public_pane_id_aliases[&old], pane);
        let generated = crate::workspace::generate_workspace_id();
        assert!(crate::workspace::public_workspace_number(&generated).unwrap() > retired_number);
        state.assert_invariants_for_test();
    }

    #[test]
    fn transfer_retargets_pending_notifications_toasts_and_remembered_focus() {
        use crate::app::state::{
            PaneFocusTarget, PendingAgentNotification, ToastKind, ToastNotification, ToastTarget,
        };
        let mut state = AppState::test_new();
        state.workspaces = vec![
            Workspace::test_new("source"),
            Workspace::test_new("destination"),
        ];
        state.active = Some(0);
        state.ensure_test_terminals();
        let pane = state.workspaces[0].tabs[0].root_pane;
        let source = state.workspaces[0].id.clone();
        let destination = state.workspaces[1].id.clone();
        state.previous_pane_focus = Some(PaneFocusTarget {
            workspace_id: source.clone(),
            pane_id: pane,
        });
        state.pending_agent_notifications.insert(
            pane,
            PendingAgentNotification {
                pane_id: pane,
                workspace_id: source.clone(),
                agent_label: "agent".into(),
                known_agent: None,
                kind: ToastKind::Finished,
                state: crate::detect::AgentState::Idle,
                deadline: std::time::Instant::now(),
            },
        );
        state.toast = Some(ToastNotification {
            kind: ToastKind::Finished,
            title: "done".into(),
            context: "task".into(),
            position: None,
            target: Some(ToastTarget {
                workspace_id: source,
                pane_id: pane,
            }),
        });
        state
            .transfer_tab(0, 0, TabTransferTarget::Workspace(1), false)
            .unwrap();
        assert_eq!(
            state.previous_pane_focus.as_ref().unwrap().workspace_id,
            destination
        );
        assert_eq!(
            state.pending_agent_notifications[&pane].workspace_id,
            destination
        );
        assert_eq!(
            state
                .toast
                .as_ref()
                .unwrap()
                .target
                .as_ref()
                .unwrap()
                .workspace_id,
            destination
        );
        state.assert_invariants_for_test();
    }

    #[test]
    fn focused_transfer_remembers_previous_unaffected_pane_and_rejection_is_atomic_on_exhaustion() {
        let mut state = AppState::test_new();
        state.workspaces = vec![
            Workspace::test_new("source"),
            Workspace::test_new("destination"),
        ];
        state.active = Some(1);
        state.selected = 1;
        state.ensure_test_terminals();
        let previous = state.current_pane_focus_target().unwrap();
        let source_root = state.workspaces[0].tabs[0].root_pane;
        let next = state.workspaces[1].next_public_pane_number;
        state.workspaces[1].next_public_pane_number = usize::MAX;
        assert!(state
            .transfer_tab(0, 0, TabTransferTarget::Workspace(1), true)
            .is_err());
        assert_eq!(state.workspaces.len(), 2);
        assert_eq!(state.workspaces[0].tabs[0].root_pane, source_root);
        assert!(state.public_pane_id_aliases.is_empty());
        state.workspaces[1].next_public_pane_number = next;
        state
            .transfer_tab(0, 0, TabTransferTarget::Workspace(1), true)
            .unwrap();
        assert_eq!(state.previous_pane_focus, Some(previous));
        state.assert_invariants_for_test();
    }
}
