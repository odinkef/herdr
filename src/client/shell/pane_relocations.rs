//! Optional public-ID relocation companions, applied before snapshot identity pruning.
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::protocol::{
    endpoint::EndpointPaneRelocations, ClientShellSnapshot, SemanticNotification,
};

use super::*;

#[derive(Clone, Debug, Default)]
pub(super) struct PaneRelocations {
    current: Option<(u64, EndpointPaneRelocations)>,
    applied: bool,
}

impl PaneRelocations {
    fn receive(&mut self, generation: u64, mut projection: EndpointPaneRelocations) {
        if self.current.as_ref().is_some_and(|(received, current)| {
            *received > generation
                || (*received == generation
                    && current.boot_id == projection.boot_id
                    && current.revision >= projection.revision)
        }) {
            return;
        }
        // Treat malformed/cyclic aliases as absent. Normalize once when the map changes,
        // never in pane-scaled rendering or repeated snapshot reconciliation.
        let aliases = &projection.aliases;
        projection.aliases = aliases
            .iter()
            .filter_map(|(retired, initial)| {
                let mut current = initial;
                let mut seen = HashSet::from([retired.as_str()]);
                while let Some(next) = aliases.get(current) {
                    if !seen.insert(current.as_str()) {
                        return None;
                    }
                    current = next;
                }
                (current != retired).then(|| (retired.clone(), current.clone()))
            })
            .collect();
        self.current = Some((generation, projection));
        self.applied = false;
    }

    fn ready(
        &mut self,
        snapshot: &ClientShellSnapshot,
        generation: Option<u64>,
    ) -> Option<BTreeMap<String, String>> {
        let (received_generation, projection) = self.current.as_ref()?;
        if self.applied
            || generation != Some(*received_generation)
            || projection.boot_id != snapshot.boot_id
            || projection.revision > snapshot.revision
        {
            return None;
        }
        self.applied = true;
        Some(projection.aliases.clone())
    }
}

pub(super) fn relocate_watermarks(
    watermarks: &mut HashMap<String, u64>,
    aliases: &BTreeMap<String, String>,
) {
    for (retired, current) in aliases {
        if let Some(sequence) = watermarks.remove(retired) {
            let destination = watermarks.entry(current.clone()).or_default();
            *destination = (*destination).max(sequence);
        }
    }
}

fn relocate_id(id: &mut String, aliases: &BTreeMap<String, String>) {
    if let Some(current) = aliases.get(id) {
        *id = current.clone();
    }
}

fn relocate_snapshot_pane_ids(
    previous: &mut ClientShellSnapshot,
    aliases: &BTreeMap<String, String>,
) {
    if let Some(id) = previous.focused_pane_id.as_mut() {
        relocate_id(id, aliases);
    }
    for pane in &mut previous.panes {
        relocate_id(&mut pane.pane_id, aliases);
    }
    for agent in &mut previous.agents {
        relocate_id(&mut agent.pane_id, aliases);
    }
    for id in &mut previous.agent_order {
        relocate_id(id, aliases);
    }
}

fn relocate_notification(
    notification: &mut SemanticNotification,
    snapshot: &ClientShellSnapshot,
    aliases: &BTreeMap<String, String>,
) {
    let Some(id) = notification.pane_id.as_mut() else {
        return;
    };
    relocate_id(id, aliases);
    if let Some(pane) = snapshot.panes.iter().find(|pane| &pane.pane_id == id) {
        notification.workspace_id = Some(pane.workspace_id.clone());
        notification.tab_id = Some(pane.tab_id.clone());
    }
}

impl ClientShellState {
    pub(crate) fn set_endpoint_pane_relocations(
        &mut self,
        endpoint_id: &ClientEndpointId,
        generation: u64,
        projection: EndpointPaneRelocations,
    ) {
        if let Some(endpoint) = self
            .endpoints
            .iter_mut()
            .find(|endpoint| &endpoint.endpoint_id == endpoint_id)
        {
            endpoint.pane_relocations.receive(generation, projection);
        }
    }

    pub(super) fn apply_endpoint_pane_relocations(
        &mut self,
        index: usize,
        snapshot: &ClientShellSnapshot,
        generation: Option<u64>,
    ) {
        let Some(aliases) = self.endpoints[index]
            .pane_relocations
            .ready(snapshot, generation)
        else {
            return;
        };
        if aliases.is_empty() {
            return;
        }
        let endpoint_id = self.endpoints[index].endpoint_id.clone();
        // A replacement boot establishes its own completion baseline. Never carry old
        // process acknowledgements onto a server which happened to reuse public IDs.
        if self.endpoints[index]
            .snapshot
            .as_deref()
            .is_some_and(|previous| previous.boot_id != snapshot.boot_id)
        {
            return;
        }
        let endpoint = &mut self.endpoints[index];
        endpoint.agent_presentation.relocate(&aliases);
        relocate_watermarks(&mut endpoint.agent_recency, &aliases);
        if let Some(previous) = endpoint.snapshot.as_deref_mut() {
            relocate_snapshot_pane_ids(previous, &aliases);
        }
        for pending in &mut self.pending_notifications {
            if pending.endpoint_id == endpoint_id {
                relocate_notification(&mut pending.event, snapshot, &aliases);
            }
        }
        for notification in &mut self.queued_notifications {
            if notification.endpoint_id == endpoint_id {
                relocate_notification(&mut notification.event, snapshot, &aliases);
            }
        }
        if let Some(notification) = self.visible_notification.as_mut() {
            if notification.endpoint_id == endpoint_id {
                relocate_notification(&mut notification.event, snapshot, &aliases);
            }
        }
        if let Some((target_endpoint, pane_id)) = self.pending_agent_reveal.as_mut() {
            if target_endpoint == &endpoint_id {
                relocate_id(pane_id, &aliases);
            }
        }
        if endpoint_id == self.active_endpoint_id {
            let affected_surface = self.pane_surface.as_ref().is_some_and(|surface| {
                surface
                    .panes
                    .iter()
                    .any(|pane| aliases.contains_key(&pane.pane_id))
            }) || self
                .snapshot
                .as_deref()
                .and_then(|snapshot| snapshot.focused_pane_id.as_ref())
                .is_some_and(|id| aliases.contains_key(id));
            if affected_surface {
                // Control and render lanes are independent. A coherent destination
                // surface may already be waiting for this snapshot: retain that new
                // authoritative pair while discarding old coordinate state.
                let current_surface = self.pane_surface.take().filter(|surface| {
                    surface.boot_id == snapshot.boot_id
                        && surface.projection_revision == snapshot.revision
                        && surface
                            .panes
                            .iter()
                            .all(|pane| !aliases.contains_key(&pane.pane_id))
                });
                let pending_surface = self.pending_pane_surface.take().filter(|surface| {
                    surface.boot_id == snapshot.boot_id
                        && surface.projection_revision == snapshot.revision
                        && surface
                            .panes
                            .iter()
                            .all(|pane| !aliases.contains_key(&pane.pane_id))
                });
                self.invalidate_relocated_pane_state();
                self.pane_surface = current_surface;
                self.pending_pane_surface = pending_surface;
            }
            self.attention.relocate(&aliases);
            if let Some(previous) = self.snapshot.as_deref_mut() {
                relocate_snapshot_pane_ids(previous, &aliases);
            }
            if let Some(previous) = self.previous_pane_id.as_mut() {
                relocate_id(previous, &aliases);
            }
        }
    }

    pub(super) fn normalize_endpoint_notification_target(
        &self,
        endpoint_id: &ClientEndpointId,
        event: &mut SemanticNotification,
    ) {
        let Some(endpoint) = self
            .endpoints
            .iter()
            .find(|endpoint| &endpoint.endpoint_id == endpoint_id)
        else {
            return;
        };
        let Some(snapshot) = endpoint.snapshot.as_deref() else {
            return;
        };
        let Some((_, projection)) = endpoint.pane_relocations.current.as_ref() else {
            return;
        };
        if endpoint.pane_relocations.applied && projection.boot_id == snapshot.boot_id {
            relocate_notification(event, snapshot, &projection.aliases);
        }
    }

    fn invalidate_relocated_pane_state(&mut self) {
        // Retire coordinate-based responses too: late scroll/link/selection replies
        // must not restore a gesture or copy text from its pre-transfer geometry.
        self.pending_requests.retain(|_, pending| {
            !matches!(
                pending.kind,
                PendingEndpointKind::SelectionCopy
                    | PendingEndpointKind::PaneScroll { .. }
                    | PendingEndpointKind::WordSelection { .. }
                    | PendingEndpointKind::PaneLinkResolve { .. }
                    | PendingEndpointKind::PaneLinkActivate { .. }
                    | PendingEndpointKind::CopyMotion { .. }
                    | PendingEndpointKind::CopySearch { .. }
            )
        });
        self.hits = ShellHitMap::default();
        self.pane_surface = None;
        self.pending_pane_surface = None;
        self.input_leases = Default::default();
        self.chrome_drag = None;
        self.workspace_press = None;
        self.tab_press = None;
        self.pane_mouse_gesture = None;
        self.link_hover = None;
        self.url_click_consumes_until_up = false;
        self.replaying_url_click = false;
        self.selection = None;
        self.last_pane_click = None;
        self.selection_autoscroll = None;
        self.selection_autoscroll_deadline = None;
        self.selection_highlight_clear_deadline = None;
        self.word_selection_gesture = None;
        self.selection_repaint_deadline = None;
        self.copy_mode = None;
        if self.mode == ClientShellMode::Copy {
            self.mode = ClientShellMode::Terminal;
        }
        self.reset_copy_pipeline();
        self.pane_scroll_in_flight.clear();
        self.pane_scroll_queued.clear();
        self.pane_scroll_targets.clear();
        self.host_mouse_pixels = None;
        self.last_composed_size = None;
        self.last_composed_at = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn projection(revision: u64, aliases: &[(&str, &str)]) -> EndpointPaneRelocations {
        EndpointPaneRelocations {
            boot_id: "boot".into(),
            revision,
            aliases: aliases
                .iter()
                .map(|(old, new)| (old.to_string(), new.to_string()))
                .collect(),
        }
    }

    #[test]
    fn relocation_companion_uses_revision_as_a_validity_barrier_and_is_applied_once() {
        let mut cache = PaneRelocations::default();
        cache.receive(3, projection(7, &[("a", "b"), ("b", "c")]));
        let mut snapshot = crate::client::shell::tests::snapshot();
        snapshot.boot_id = "boot".into();
        snapshot.revision = 6;
        assert!(cache.ready(&snapshot, Some(3)).is_none());
        snapshot.revision = 9; // The transport coalesced/skipped the barrier snapshot.
        assert!(cache.ready(&snapshot, Some(2)).is_none());
        snapshot.boot_id = "replacement".into();
        assert!(cache.ready(&snapshot, Some(3)).is_none());
        snapshot.boot_id = "boot".into();
        let aliases = cache.ready(&snapshot, Some(3)).unwrap();
        assert_eq!(aliases["a"], "c");
        assert_eq!(aliases["b"], "c");
        assert!(cache.ready(&snapshot, Some(3)).is_none());
        cache.receive(3, projection(6, &[("a", "old")]));
        assert!(cache.ready(&snapshot, Some(3)).is_none());
        cache.receive(4, projection(1, &[("a", "c"), ("b", "c")]));
        snapshot.revision = 1;
        assert!(cache.ready(&snapshot, Some(4)).is_some());
    }

    #[test]
    fn relocation_map_rejects_cycles_and_merges_watermarks_by_maximum() {
        let mut cache = PaneRelocations::default();
        cache.receive(
            1,
            projection(1, &[("a", "b"), ("b", "a"), ("x", "z"), ("y", "z")]),
        );
        let mut snapshot = crate::client::shell::tests::snapshot();
        snapshot.boot_id = "boot".into();
        let aliases = cache.ready(&snapshot, Some(1)).unwrap();
        assert!(!aliases.contains_key("a"));
        let mut sequences = [("x".into(), 9), ("y".into(), 3), ("z".into(), 5)]
            .into_iter()
            .collect();
        relocate_watermarks(&mut sequences, &aliases);
        assert_eq!(sequences, HashMap::from([("z".into(), 9)]));
    }
    #[test]
    fn relocation_precedes_client_pruning_and_preserves_recency_and_notification_targets() {
        let mut state = ClientShellState::new(ClientShellConfig::from_config(
            &crate::config::Config::default(),
        ));
        let mut before = crate::client::shell::tests::snapshot();
        before.agents.push(crate::protocol::ClientShellAgent {
            pane_id: "pane_1".into(),
            workspace_id: "ws_1".into(),
            tab_id: "tab_1".into(),
            name: None,
            display_agent: None,
            agent: None,
            title: None,
            terminal_title: None,
            terminal_title_stripped: None,
            agent_status: crate::api::schema::AgentStatus::Working,
            state_change_seq: 1,
            state_labels: Vec::new(),
            tokens: Vec::new(),
            focused: true,
        });
        state.set_endpoint_snapshot_for_generation(
            &ClientEndpointId::Local,
            1,
            Box::new(before.clone()),
        );
        before.revision = 2;
        before.agents[0].agent_status = crate::api::schema::AgentStatus::Idle;
        before.agents[0].state_change_seq = 2;
        state.set_endpoint_snapshot_for_generation(
            &ClientEndpointId::Local,
            1,
            Box::new(before.clone()),
        );
        let recency = state.endpoints[0].agent_recency["pane_1"];
        state.pane_surface = Some(crate::client::shell::tests::surface());
        state.pending_requests.insert(
            "scroll".into(),
            PendingEndpointRequest {
                boot_id: "boot-1".into(),
                method_name: "pane.scroll".into(),
                confirmation_workspace_id: None,
                kind: PendingEndpointKind::PaneScroll {
                    pane_id: "pane_1".into(),
                    serial: 1,
                },
            },
        );
        state.visible_notification = Some(ClientVisibleNotification {
            endpoint_id: ClientEndpointId::Local,
            deadline: std::time::Instant::now(),
            event: SemanticNotification {
                kind: SemanticNotificationKind::Finished,
                title: "done".into(),
                body: None,
                sound: None,
                agent: None,
                workspace_id: Some("ws_1".into()),
                tab_id: Some("tab_1".into()),
                pane_id: Some("pane_1".into()),
                position: None,
            },
        });
        state.set_endpoint_pane_relocations(
            &ClientEndpointId::Local,
            2,
            EndpointPaneRelocations {
                boot_id: "boot-1".into(),
                revision: 1,
                aliases: BTreeMap::from([("pane_1".into(), "new-pane".into())]),
            },
        );
        let mut after = before;
        after.revision = 1; // Same boot, fresh connection's projection counter.
        after.focused_pane_id = Some("new-pane".into());
        after.panes[0].pane_id = "new-pane".into();
        after.panes[0].workspace_id = "destination".into();
        after.panes[0].tab_id = "new-tab".into();
        after.agents[0].pane_id = "new-pane".into();
        state.set_endpoint_snapshot_for_generation(&ClientEndpointId::Local, 2, Box::new(after));
        assert_eq!(state.endpoints[0].agent_recency["new-pane"], recency);
        assert!(!state.endpoints[0].agent_recency.contains_key("pane_1"));
        assert_eq!(
            state.snapshot.as_ref().unwrap().agents[0].agent_status,
            crate::api::schema::AgentStatus::Done
        );
        assert!(state.pane_surface.is_none());
        assert!(!state.pending_requests.contains_key("scroll"));
        let notification = &state.visible_notification.as_ref().unwrap().event;
        assert_eq!(notification.pane_id.as_deref(), Some("new-pane"));
        assert_eq!(notification.workspace_id.as_deref(), Some("destination"));
        assert_eq!(notification.tab_id.as_deref(), Some("new-tab"));
    }
    #[test]
    fn relocation_keeps_an_authoritative_destination_surface_that_arrived_before_its_snapshot() {
        let mut state = ClientShellState::new(ClientShellConfig::from_config(
            &crate::config::Config::default(),
        ));
        let mut snapshot = crate::client::shell::tests::snapshot();
        state.set_endpoint_snapshot_for_generation(
            &ClientEndpointId::Local,
            1,
            Box::new(snapshot.clone()),
        );
        state.pane_surface = Some(crate::client::shell::tests::surface());
        let mut destination = crate::client::shell::tests::surface();
        destination.projection_revision = 2;
        destination.panes[0].pane_id = "new-pane".into();
        state.pending_pane_surface = Some(destination);
        state.set_endpoint_pane_relocations(
            &ClientEndpointId::Local,
            1,
            EndpointPaneRelocations {
                boot_id: "boot-1".into(),
                revision: 2,
                aliases: BTreeMap::from([("pane_1".into(), "new-pane".into())]),
            },
        );
        snapshot.revision = 2;
        snapshot.focused_pane_id = Some("new-pane".into());
        snapshot.panes[0].pane_id = "new-pane".into();
        state.set_endpoint_snapshot_for_generation(&ClientEndpointId::Local, 1, Box::new(snapshot));
        assert_eq!(
            state.pane_surface.as_ref().unwrap().panes[0].pane_id,
            "new-pane"
        );
        assert_eq!(state.pane_surface.as_ref().unwrap().projection_revision, 2);
    }
}
