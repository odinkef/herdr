use std::collections::{HashMap, HashSet, VecDeque};

use crate::api::schema::{AgentStatus, Method, PaneTarget};
use crate::protocol::ClientShellSnapshot;

use super::{ClientShellInput, ClientShellState, PendingEndpointKind};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Target {
    pane_id: String,
    sequence: u64,
}

/// Client presentation state only; no runtime or wire-protocol changes.
#[derive(Debug, Default)]
pub(super) struct AttentionCycle {
    remaining: VecDeque<Target>,
    visited: HashSet<Target>,
    pending_focus: VecDeque<String>,
}

impl AttentionCycle {
    pub(super) fn relocate(&mut self, aliases: &std::collections::BTreeMap<String, String>) {
        for target in &mut self.remaining {
            if let Some(current) = aliases.get(&target.pane_id) {
                target.pane_id = current.clone();
            }
        }
        self.visited = self
            .visited
            .drain()
            .map(|mut target| {
                if let Some(current) = aliases.get(&target.pane_id) {
                    target.pane_id = current.clone();
                }
                target
            })
            .collect();
        for pane in &mut self.pending_focus {
            if let Some(current) = aliases.get(pane) {
                *pane = current.clone();
            }
        }
    }

    pub(super) fn observe_focus(&mut self, previous: Option<&str>, current: Option<&str>) {
        if previous == current {
            return;
        }
        if let Some(index) = self
            .pending_focus
            .iter()
            .position(|pane| Some(pane.as_str()) == current)
        {
            // A snapshot can coalesce several rapid shortcut presses.
            self.pending_focus.drain(..=index);
        } else {
            *self = Self::default();
        }
    }

    fn next(&mut self, snapshot: &ClientShellSnapshot) -> Option<String> {
        let candidates = candidates(snapshot);
        let eligible: HashSet<_> = candidates.iter().collect();
        self.remaining.retain(|target| eligible.contains(target));
        self.visited.retain(|target| eligible.contains(target));
        let queued: HashSet<_> = self.remaining.iter().cloned().collect();
        for candidate in &candidates {
            if !self.visited.contains(candidate) && !queued.contains(candidate) {
                self.remaining.push_back(candidate.clone());
            }
        }
        let focused = self
            .pending_focus
            .back()
            .map(String::as_str)
            .or(snapshot.focused_pane_id.as_deref());
        let other = |target: &Target| Some(target.pane_id.as_str()) != focused;
        if !self.remaining.iter().any(other) {
            // Only a focused target remains, or this cycle is exhausted.
            self.visited.clear();
            self.remaining = candidates.into();
        }
        let index = self.remaining.iter().position(other)?;
        let target = self.remaining.remove(index)?;
        self.visited.insert(target.clone());
        self.pending_focus.push_back(target.pane_id.clone());
        Some(target.pane_id)
    }
}

/// Compute order only on shortcut input, never in a render or pane-output loop.
fn candidates(snapshot: &ClientShellSnapshot) -> Vec<Target> {
    let workspaces: HashMap<_, _> = snapshot
        .workspaces
        .iter()
        .enumerate()
        .map(|(index, workspace)| (workspace.workspace_id.as_str(), index))
        .collect();
    let tabs: HashMap<_, _> = snapshot
        .tabs
        .iter()
        .enumerate()
        .map(|(index, tab)| (tab.tab_id.as_str(), (tab.workspace_id.as_str(), index)))
        .collect();
    let agents: HashMap<_, _> = snapshot
        .agents
        .iter()
        .filter(|agent| matches!(agent.agent_status, AgentStatus::Blocked | AgentStatus::Done))
        .map(|agent| (agent.pane_id.as_str(), agent))
        .collect();
    let mut ordered = snapshot
        .panes
        .iter()
        .enumerate()
        .filter_map(|(pane_index, pane)| {
            let agent = agents.get(pane.pane_id.as_str())?;
            let workspace_index = workspaces.get(pane.workspace_id.as_str())?;
            let (tab_workspace, tab_index) = tabs.get(pane.tab_id.as_str())?;
            if *tab_workspace != pane.workspace_id
                || agent.workspace_id != pane.workspace_id
                || agent.tab_id != pane.tab_id
            {
                return None;
            }
            Some((
                (
                    agent.agent_status != AgentStatus::Blocked,
                    *workspace_index,
                    *tab_index,
                    pane_index,
                ),
                Target {
                    pane_id: pane.pane_id.clone(),
                    sequence: agent.state_change_seq,
                },
            ))
        })
        .collect::<Vec<_>>();
    ordered.sort_by_key(|(order, _)| *order);
    ordered.into_iter().map(|(_, target)| target).collect()
}

impl ClientShellState {
    pub(super) fn focus_next_attention(&mut self, outcome: &mut ClientShellInput) {
        let Some(snapshot) = self.snapshot.as_deref() else {
            return;
        };
        let Some(pane_id) = self.attention.next(snapshot) else {
            return;
        };
        if !self.push_endpoint_method_with_kind(
            Method::PaneFocus(PaneTarget { pane_id }),
            PendingEndpointKind::AttentionFocus,
            outcome,
        ) {
            self.attention = Default::default();
        }
    }
}

#[cfg(test)]
#[path = "attention_tests.rs"]
mod tests;
