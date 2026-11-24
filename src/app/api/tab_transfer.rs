use crate::api::schema::{
    EventData, EventEnvelope, EventKind, ResponseResult, TabTransferDestination, TabTransferParams,
};
use crate::app::tab_transfer::TabTransferTarget;
use crate::app::App;

use super::responses::{encode_error, encode_success};

impl App {
    pub(super) fn handle_tab_transfer(&mut self, id: String, params: TabTransferParams) -> String {
        let Some((source_workspace, source_tab)) = self.parse_tab_id(&params.tab_id) else {
            return encode_error(
                id,
                "tab_not_found",
                format!("tab {} not found", params.tab_id),
            );
        };
        let destination = match params.destination {
            TabTransferDestination::Workspace { workspace_id } => {
                let Some(index) = self.parse_workspace_id(&workspace_id) else {
                    return encode_error(
                        id,
                        "workspace_not_found",
                        format!("workspace {workspace_id} not found"),
                    );
                };
                TabTransferTarget::Workspace(index)
            }
            TabTransferDestination::NewWorkspace { label } => {
                let focused = self.state.workspaces[source_workspace].tabs[source_tab]
                    .layout
                    .focused();
                let initial_cwd = self.launch_cwd_for_pane_in_workspace(source_workspace, focused);
                TabTransferTarget::NewWorkspace { label, initial_cwd }
            }
        };
        let source_info = self.workspace_info(source_workspace);
        let commit =
            match self
                .state
                .transfer_tab(source_workspace, source_tab, destination, params.focus)
            {
                Ok(commit) => commit,
                Err(message) => return encode_error(id, "tab_transfer_failed", message),
            };
        // Overlay records store vector positions. Rebind them by stable raw pane
        // identity before any deferred command completion can inspect ownership.
        if commit.changed {
            for (pane, overlay) in &mut self.overlay_panes {
                if let Some((ws, tab)) =
                    self.state
                        .workspaces
                        .iter()
                        .enumerate()
                        .find_map(|(ws, workspace)| {
                            workspace
                                .find_tab_index_for_pane(*pane)
                                .map(|tab| (ws, tab))
                        })
                {
                    overlay.ws_idx = ws;
                    overlay.tab_idx = tab;
                }
            }
        }
        let ws = commit.workspace_index;
        let tab_index = commit.tab_index;
        let tab = self
            .tab_info(ws, tab_index)
            .expect("validated transferred tab has public identity");
        let layout = self
            .pane_layout_snapshot(ws, tab_index)
            .expect("validated transferred tab has layout");
        let created_workspace = commit.created_workspace.then(|| self.workspace_info(ws));
        let removed_workspace = commit.removed_workspace.then_some(source_info);
        if commit.changed {
            self.schedule_session_save();
            self.emit_event(EventEnvelope {
                event: EventKind::TabClosed,
                data: EventData::TabClosed {
                    tab_id: commit.previous_tab_id.clone(),
                    workspace_id: commit.previous_workspace_id.clone(),
                },
            });
            if let Some(workspace) = &removed_workspace {
                self.emit_event(EventEnvelope {
                    event: EventKind::WorkspaceClosed,
                    data: EventData::WorkspaceClosed {
                        workspace_id: workspace.workspace_id.clone(),
                        workspace: Some(workspace.clone()),
                    },
                });
            }
            if let Some(workspace) = &created_workspace {
                self.emit_event(EventEnvelope {
                    event: EventKind::WorkspaceCreated,
                    data: EventData::WorkspaceCreated {
                        workspace: workspace.clone(),
                    },
                });
            }
            self.emit_event(EventEnvelope {
                event: EventKind::TabCreated,
                data: EventData::TabCreated { tab: tab.clone() },
            });
            for pane_id in &commit.pane_ids {
                let pane = self
                    .pane_info(ws, *pane_id)
                    .expect("transferred pane remains attached");
                let previous_pane_id = commit
                    .pane_id_map
                    .iter()
                    .find_map(|(old, new)| (new == &pane.pane_id).then(|| old.clone()))
                    .expect("every transferred pane has a mapping");
                self.emit_event(EventEnvelope {
                    event: EventKind::PaneMoved,
                    data: EventData::PaneMoved {
                        previous_pane_id,
                        previous_workspace_id: commit.previous_workspace_id.clone(),
                        previous_tab_id: commit.previous_tab_id.clone(),
                        pane: Box::new(pane),
                        created_workspace: created_workspace.clone(),
                        created_tab: Some(tab.clone()),
                        closed_workspace_id: removed_workspace
                            .as_ref()
                            .map(|ws| ws.workspace_id.clone()),
                        closed_tab_id: Some(commit.previous_tab_id.clone()),
                    },
                });
            }
            self.emit_layout_updated_snapshot(layout.clone());
        }
        encode_success(
            id,
            ResponseResult::TabTransferred {
                changed: commit.changed,
                previous_workspace_id: commit.previous_workspace_id,
                previous_tab_id: commit.previous_tab_id,
                tab,
                layout,
                pane_id_map: commit.pane_id_map,
                created_workspace,
                removed_workspace,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::schema::{ErrorResponse, Method, PaneCurrentParams, Request, SuccessResponse};
    use crate::config::Config;
    use crate::workspace::Workspace;

    fn app() -> (App, crate::api::EventHub) {
        let hub = crate::api::EventHub::default();
        let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (
            App::new(
                &Config::default(),
                crate::app::AppPolicy::TEST,
                None,
                rx,
                hub.clone(),
            ),
            hub,
        )
    }

    #[test]
    fn transfer_emits_lifecycle_and_movement_after_commit_without_exit_or_reorder_events() {
        let (mut app, hub) = app();
        let mut source = Workspace::test_new("source");
        let second = source.test_split(ratatui::layout::Direction::Horizontal);
        source.tabs[0].layout.focus_pane(second);
        source.tabs[0].zoomed = true;
        source.tabs[0].custom_name = Some("running".into());
        app.state.workspaces = vec![source, Workspace::test_new("destination")];
        app.state.active = Some(0);
        app.state.ensure_test_terminals();
        let old_tab = app.public_tab_id(0, 0).unwrap();
        let old_pane = app.public_pane_id(0, second).unwrap();
        let destination_id = app.public_workspace_id(1);
        let response = app.handle_tab_transfer(
            "move".into(),
            TabTransferParams {
                tab_id: old_tab.clone(),
                destination: TabTransferDestination::Workspace {
                    workspace_id: destination_id.clone(),
                },
                focus: true,
            },
        );
        let response: SuccessResponse = serde_json::from_str(&response).unwrap();
        let ResponseResult::TabTransferred {
            changed,
            tab,
            layout,
            pane_id_map,
            removed_workspace,
            ..
        } = response.result
        else {
            panic!("expected transfer result");
        };
        assert!(changed);
        assert_eq!(tab.workspace_id, destination_id);
        assert_eq!(tab.label, "running");
        assert_eq!(layout.focused_pane_id, pane_id_map[&old_pane]);
        assert!(layout.zoomed);
        assert!(removed_workspace.is_some());
        assert_eq!(
            app.parse_pane_id(&old_pane),
            app.parse_pane_id(&pane_id_map[&old_pane])
        );
        let events = hub.events_after(0);
        assert_eq!(
            events
                .iter()
                .map(|(_, event)| event.event)
                .collect::<Vec<_>>(),
            vec![
                EventKind::TabClosed,
                EventKind::WorkspaceClosed,
                EventKind::TabCreated,
                EventKind::PaneMoved,
                EventKind::PaneMoved,
                EventKind::LayoutUpdated,
            ]
        );
        // An inherited caller pane environment still resolves to its current tab.
        let current = app.handle_api_request(Request {
            id: "caller".into(),
            method: Method::PaneCurrent(PaneCurrentParams {
                caller_pane_id: Some(old_pane),
            }),
        });
        let current: SuccessResponse = serde_json::from_str(&current).unwrap();
        let ResponseResult::PaneCurrent { pane } = current.result else {
            panic!("expected current pane");
        };
        assert_eq!(pane.tab_id, tab.tab_id);
        app.state.assert_invariants_for_test();
    }

    #[test]
    fn transfer_rejection_and_same_workspace_produce_no_events_or_partial_mutations() {
        let (mut app, hub) = app();
        app.state.workspaces = vec![Workspace::test_adversarial_identity_state()];
        app.state.active = Some(0);
        app.state.ensure_test_terminals();
        let tab_id = app.public_tab_id(0, 0).unwrap();
        let original_root = app.state.workspaces[0].tabs[0].root_pane;
        let bad = app.handle_tab_transfer(
            "bad".into(),
            TabTransferParams {
                tab_id: tab_id.clone(),
                destination: TabTransferDestination::Workspace {
                    workspace_id: "missing".into(),
                },
                focus: true,
            },
        );
        assert_eq!(
            serde_json::from_str::<ErrorResponse>(&bad)
                .unwrap()
                .error
                .code,
            "workspace_not_found"
        );
        let noop = app.handle_tab_transfer(
            "noop".into(),
            TabTransferParams {
                tab_id,
                destination: TabTransferDestination::Workspace {
                    workspace_id: app.public_workspace_id(0),
                },
                focus: true,
            },
        );
        let noop: SuccessResponse = serde_json::from_str(&noop).unwrap();
        assert!(matches!(
            noop.result,
            ResponseResult::TabTransferred { changed: false, .. }
        ));
        assert_eq!(app.state.workspaces[0].tabs[0].root_pane, original_root);
        assert!(hub.events_after(0).is_empty());
        app.state.assert_invariants_for_test();
    }

    #[test]
    fn tab_transfer_json_default_follows_and_destinations_are_explicit() {
        let request: Request = serde_json::from_value(serde_json::json!({
            "id":"transfer", "method":"tab.transfer", "params": {
                "tab_id":"w1:t1", "destination": {"type":"new_workspace", "label":"new"}
            }
        }))
        .unwrap();
        let Method::TabTransfer(params) = request.method else {
            panic!("expected transfer");
        };
        assert!(params.focus);
        assert_eq!(
            params.destination,
            TabTransferDestination::NewWorkspace {
                label: Some("new".into())
            }
        );
    }
}
