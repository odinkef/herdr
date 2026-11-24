use super::*;

/// A public tab number is scoped to one server boot and connection. Never infer
/// a transfer source from focus after opening a menu or starting a drag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ClientTabTransferSource {
    pub(super) endpoint_id: ClientEndpointId,
    pub(super) boot_id: String,
    pub(super) generation: Option<u64>,
    pub(super) connection_epoch: u64,
    pub(super) tab_id: String,
    pub(super) workspace_id: String,
}

#[derive(Debug)]
pub(super) struct ClientTabTransferOverlay {
    pub(super) source: ClientTabTransferSource,
    pub(super) query: TextEditor,
    pub(super) selected: usize,
    pub(super) submitting: bool,
    pub(super) error: Option<String>,
}

impl ClientTabTransferOverlay {
    pub(super) fn entries(&self, snapshot: &ClientShellSnapshot) -> Vec<ClientWorktreeOpenEntry> {
        let mut entries = snapshot
            .workspaces
            .iter()
            .filter(|workspace| {
                workspace.workspace_id != self.source.workspace_id
                    && workspace
                        .label
                        .to_lowercase()
                        .contains(&self.query.trim().to_lowercase())
            })
            .map(|workspace| ClientWorktreeOpenEntry {
                path: workspace.workspace_id.clone(),
                label: workspace.label.clone(),
                branch: Some(String::new()),
                is_linked_worktree: false,
                is_detached: false,
                open_workspace_id: None,
            })
            .collect::<Vec<_>>();
        // The create row remains below real matches for every query.
        entries.push(ClientWorktreeOpenEntry {
            path: String::new(),
            label: if self.query.is_empty() {
                "New space…".into()
            } else {
                format!("New space… {}", self.query)
            },
            branch: Some(String::new()),
            is_linked_worktree: false,
            is_detached: false,
            open_workspace_id: None,
        });
        entries
    }

    pub(super) fn list(&self, snapshot: &ClientShellSnapshot) -> ClientWorktreeOpenOverlay {
        let mut list = ClientWorktreeOpenOverlay {
            source_workspace_id: self.source.workspace_id.clone(),
            entries: self.entries(snapshot),
            selected: self.selected,
            query: self.query.clone(),
            search_focused: true,
            error: self.error.clone(),
            opening: self.submitting,
        };
        list.selected = list.selected_entry_index().unwrap_or(0);
        list
    }
}

impl ClientShellState {
    pub(super) fn invalidate_tab_transfer_targets(&mut self) {
        self.tab_transfer_epoch = self.tab_transfer_epoch.saturating_add(1);
        if matches!(&self.chrome_drag, Some(ClientChromeDrag::Tab { .. })) {
            self.chrome_drag = None;
        }
        self.tab_press = None;
        if let Some(ClientShellOverlay::TabTransfer(picker)) = self.overlay.as_mut() {
            let message = if picker.submitting {
                "The connection changed. The transfer may have completed; check the tab's location, then cancel and reopen Move to space…"
            } else {
                "The connection changed. Cancel and reopen Move to space…"
            };
            picker.submitting = false;
            picker.error = Some(message.into());
        }
    }

    fn supports_tab_transfer(&self) -> bool {
        self.endpoints
            .iter()
            .find(|e| e.endpoint_id == self.active_endpoint_id)
            .and_then(|e| e.methods.as_ref())
            .is_some_and(|methods| methods.contains("tab.transfer"))
    }

    pub(super) fn capture_tab_transfer_source(
        &self,
        tab_id: &str,
    ) -> Option<ClientTabTransferSource> {
        let snapshot = self.snapshot.as_deref()?;
        let tab = snapshot.tabs.iter().find(|tab| tab.tab_id == tab_id)?;
        Some(ClientTabTransferSource {
            endpoint_id: self.active_endpoint_id.clone(),
            boot_id: snapshot.boot_id.clone(),
            generation: self.active_snapshot_generation,
            connection_epoch: self.tab_transfer_epoch,
            tab_id: tab.tab_id.clone(),
            workspace_id: tab.workspace_id.clone(),
        })
    }

    fn tab_transfer_connection_is_current(&self, source: &ClientTabTransferSource) -> bool {
        source.endpoint_id == self.active_endpoint_id
            && source.connection_epoch == self.tab_transfer_epoch
            && source.generation == self.active_snapshot_generation
            && self.endpoint_is_online(&source.endpoint_id)
            && self
                .snapshot
                .as_deref()
                .is_some_and(|s| s.boot_id == source.boot_id)
    }

    pub(super) fn tab_transfer_source_is_current(&self, source: &ClientTabTransferSource) -> bool {
        self.tab_transfer_connection_is_current(source)
            && self.snapshot.as_deref().is_some_and(|s| {
                s.tabs.iter().any(|tab| {
                    tab.tab_id == source.tab_id && tab.workspace_id == source.workspace_id
                })
            })
    }

    pub(super) fn open_tab_transfer_picker(
        &mut self,
        tab_id: String,
        outcome: &mut ClientShellInput,
    ) {
        let Some(source) = self.capture_tab_transfer_source(&tab_id) else {
            return;
        };
        if !self.supports_tab_transfer() {
            outcome.repaint |= self.push_endpoint_notice(
                ClientEndpointNoticeKind::Unsupported, "tab.transfer", "Action unavailable",
                "This server does not support tab.transfer yet. Update and restart it to move tabs between spaces.");
            return;
        }
        if self.pending_requests.values().any(|pending| matches!(&pending.kind,
            PendingEndpointKind::TabTransfer { source: pending_source } if pending_source == &source)) {
            return;
        }
        self.overlay = Some(ClientShellOverlay::TabTransfer(ClientTabTransferOverlay {
            source,
            query: TextEditor::default(),
            selected: 0,
            submitting: false,
            error: None,
        }));
        outcome.repaint = true;
    }

    pub(super) fn move_tab_transfer_selection(&mut self, delta: isize) {
        let Some(snapshot) = self.snapshot.as_deref() else {
            return;
        };
        let Some(ClientShellOverlay::TabTransfer(picker)) = self.overlay.as_mut() else {
            return;
        };
        if picker.submitting {
            return;
        }
        let filtered = picker.list(snapshot).filtered_indices();
        let current = filtered
            .iter()
            .position(|i| *i == picker.selected)
            .unwrap_or(0);
        let next =
            (current as isize + delta).clamp(0, filtered.len().saturating_sub(1) as isize) as usize;
        picker.selected = filtered.get(next).copied().unwrap_or(0);
    }

    pub(super) fn submit_tab_transfer_picker(&mut self, outcome: &mut ClientShellInput) {
        let Some(snapshot) = self.snapshot.as_deref() else {
            return;
        };
        let Some(ClientShellOverlay::TabTransfer(picker)) = self.overlay.as_ref() else {
            return;
        };
        if picker.submitting {
            return;
        }
        let list = picker.list(snapshot);
        let Some(index) = list.selected_entry_index() else {
            return;
        };
        let source = picker.source.clone();
        if !self.tab_transfer_source_is_current(&source) {
            self.set_tab_transfer_error(
                "The source tab or connection changed. Cancel and reopen Move to space…".into(),
            );
            outcome.repaint = true;
            return;
        }
        if list.entries[index].path.is_empty() {
            self.overlay = Some(ClientShellOverlay::Rename(ClientRenameOverlay {
                title: "new space",
                input: TextEditor::new(&picker.query, false),
                target: ClientRenameTarget::TransferNewWorkspace { source },
            }));
            outcome.repaint = true;
        } else {
            self.submit_tab_transfer(
                source,
                crate::api::schema::TabTransferDestination::Workspace {
                    workspace_id: list.entries[index].path.clone(),
                },
                outcome,
            );
        }
    }

    pub(super) fn submit_tab_transfer(
        &mut self,
        source: ClientTabTransferSource,
        destination: crate::api::schema::TabTransferDestination,
        outcome: &mut ClientShellInput,
    ) {
        if !self.supports_tab_transfer() {
            self.set_tab_transfer_error(
                "This server does not advertise tab.transfer. Update and restart it to move tabs."
                    .into(),
            );
            outcome.repaint = true;
            return;
        }
        if !self.tab_transfer_source_is_current(&source) {
            self.set_tab_transfer_error(
                "The source tab or connection changed. Cancel and reopen Move to space…".into(),
            );
            outcome.repaint = true;
            return;
        }
        if self.pending_requests.values().any(|pending| matches!(&pending.kind,
            PendingEndpointKind::TabTransfer { source: pending_source } if pending_source == &source)) {
            return;
        }
        if let crate::api::schema::TabTransferDestination::Workspace { workspace_id } = &destination
        {
            if workspace_id == &source.workspace_id
                || !self
                    .snapshot
                    .as_deref()
                    .is_some_and(|s| s.workspaces.iter().any(|w| &w.workspace_id == workspace_id))
            {
                return;
            }
        }
        if !matches!(&self.overlay, Some(ClientShellOverlay::TabTransfer(p)) if p.source == source)
        {
            let query = match &destination {
                crate::api::schema::TabTransferDestination::NewWorkspace { label } => {
                    TextEditor::new(label.as_deref().unwrap_or_default(), false)
                }
                _ => TextEditor::default(),
            };
            self.overlay = Some(ClientShellOverlay::TabTransfer(ClientTabTransferOverlay {
                source: source.clone(),
                query,
                selected: 0,
                submitting: false,
                error: None,
            }));
        }
        if let Some(ClientShellOverlay::TabTransfer(picker)) = self.overlay.as_mut() {
            picker.submitting = true;
            picker.error = None;
        }
        if !self.push_endpoint_method_with_kind(
            crate::api::schema::Method::TabTransfer(crate::api::schema::TabTransferParams {
                tab_id: source.tab_id.clone(),
                destination,
                focus: true,
            }),
            PendingEndpointKind::TabTransfer { source },
            outcome,
        ) {
            if let Some(ClientShellOverlay::TabTransfer(picker)) = self.overlay.as_mut() {
                picker.submitting = false;
            }
        }
        outcome.repaint = true;
    }

    fn set_tab_transfer_error(&mut self, error: String) {
        if let Some(ClientShellOverlay::TabTransfer(picker)) = self.overlay.as_mut() {
            picker.submitting = false;
            picker.error = Some(error);
        } else {
            self.set_endpoint_error(&error);
        }
    }

    pub(super) fn complete_tab_transfer(
        &mut self,
        source: &ClientTabTransferSource,
        result: Result<crate::api::schema::ResponseResult, ClientShellEndpointError>,
    ) -> (bool, Vec<ClientShellAction>) {
        if !self.tab_transfer_connection_is_current(source) {
            return (false, Vec::new());
        }
        match result {
            Ok(crate::api::schema::ResponseResult::TabTransferred { .. }) => {
                if matches!(&self.overlay, Some(ClientShellOverlay::TabTransfer(p)) if p.source == *source)
                {
                    self.overlay = None;
                }
            }
            Ok(_) => self.set_tab_transfer_error(
                "Unexpected transfer response. Check the tab's location before retrying.".into(),
            ),
            Err(error) => self.set_tab_transfer_error(format!(
                "{} Check the tab's location before retrying.",
                error.message
            )),
        }
        (true, Vec::new())
    }

    pub(super) fn insert_tab_transfer_text(&mut self, text: &str) -> bool {
        let Some(ClientShellOverlay::TabTransfer(picker)) = self.overlay.as_mut() else {
            return false;
        };
        if !picker.submitting {
            picker.query.insert(text);
            picker.selected = 0;
        }
        true
    }

    pub(super) fn route_tab_transfer_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) -> bool {
        let Some(ClientShellOverlay::TabTransfer(picker)) = self.overlay.as_mut() else {
            return false;
        };
        if picker.submitting {
            return true;
        }
        if picker.query.handle_key(key).is_some() {
            picker.selected = 0;
            outcome.repaint = true;
            return true;
        }
        match key.code {
            KeyCode::Esc => {
                self.overlay = None;
                outcome.repaint = true;
            }
            KeyCode::Enter => self.submit_tab_transfer_picker(outcome),
            KeyCode::Up => {
                self.move_tab_transfer_selection(-1);
                outcome.repaint = true;
            }
            KeyCode::Down => {
                self.move_tab_transfer_selection(1);
                outcome.repaint = true;
            }
            _ => {}
        }
        true
    }

    pub(super) fn tab_transfer_drop_target(
        &self,
        source: &ClientTabTransferSource,
        point: (u16, u16),
    ) -> Option<String> {
        if !self.tab_transfer_source_is_current(source) {
            return None;
        }
        if !self.supports_tab_transfer() {
            return None;
        }
        self.hits
            .workspaces
            .iter()
            .find(|hit| {
                hit.endpoint_id == source.endpoint_id
                    && hit.workspace_id != source.workspace_id
                    && super::contains(hit.rect, point)
            })
            .map(|hit| hit.workspace_id.clone())
    }
}
