use super::*;
use crate::api::schema::{Method, TabTransferDestination};

fn transfer_state() -> ClientShellState {
    let mut projected = snapshot();
    let mut second = projected.tabs[0].clone();
    second.tab_id = "tab_2".into();
    second.focused = false;
    second.number = 2;
    projected.tabs.push(second);
    for (number, label) in [(2, "Alpine"), (3, "Beta")] {
        let mut workspace = projected.workspaces[0].clone();
        workspace.workspace_id = format!("ws_{number}");
        workspace.number = number;
        workspace.label = label.into();
        workspace.focused = false;
        projected.workspaces.push(workspace);
    }
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(projected));
    state.set_pane_surface(surface());
    state.set_endpoint_methods(Some(vec![
        "tab.transfer".into(),
        "tab.move".into(),
        "tab.focus".into(),
    ]));
    state.compose(106, 30).unwrap();
    state
}

fn mouse(
    state: &mut ClientShellState,
    kind: MouseEventKind,
    point: (u16, u16),
) -> ClientShellInput {
    state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind,
        column: point.0,
        row: point.1,
        modifiers: KeyModifiers::empty(),
    })])
}

#[test]
fn transfer_menu_captures_inactive_source_and_cancel_never_focuses_it() {
    let mut state = transfer_state();
    state.open_tab_context_menu("tab_2".into(), 40, 1);
    let Some(ClientShellOverlay::ContextMenu(menu)) = &state.overlay else {
        panic!("menu")
    };
    let index = menu
        .items()
        .iter()
        .position(|item| item.action == ClientContextMenuAction::MoveTabToWorkspace)
        .unwrap();
    let mut outcome = ClientShellInput::default();
    state.activate_context_menu_item(index, &mut outcome);
    assert!(outcome.actions.is_empty());
    let Some(ClientShellOverlay::TabTransfer(picker)) = &state.overlay else {
        panic!("picker")
    };
    assert_eq!(picker.source.tab_id, "tab_2");
    assert_eq!(
        state.snapshot.as_ref().unwrap().focused_tab_id.as_deref(),
        Some("tab_1")
    );
    let cancelled = state.handle_input_bytes(b"\x1b");
    assert!(cancelled.actions.is_empty());
    assert!(state.overlay.is_none());
}

#[test]
fn transfer_picker_filters_names_and_seeds_new_space_naming_dialog() {
    let mut state = transfer_state();
    let mut outcome = ClientShellInput::default();
    state.open_tab_transfer_picker("tab_2".into(), &mut outcome);
    assert!(state.insert_overlay_text("alp"));
    let Some(ClientShellOverlay::TabTransfer(picker)) = &state.overlay else {
        panic!("picker")
    };
    let list = picker.list(state.snapshot.as_deref().unwrap());
    assert_eq!(list.entries.len(), 2);
    assert_eq!(list.entries[0].label, "Alpine");
    assert!(list.entries[1].label.starts_with("New space…"));
    state.move_tab_transfer_selection(1);
    state.submit_tab_transfer_picker(&mut outcome);
    let Some(ClientShellOverlay::Rename(rename)) = &state.overlay else {
        panic!("naming")
    };
    assert_eq!(rename.input.as_str(), "alp");
    assert!(
        matches!(&rename.target, ClientRenameTarget::TransferNewWorkspace { source } if source.tab_id == "tab_2")
    );
    assert!(outcome.actions.is_empty());
    state.save_rename_overlay(&mut outcome);
    assert!(
        matches!(&outcome.actions[..], [ClientShellAction::Endpoint { request, .. }]
        if matches!(&request.method, Method::TabTransfer(p) if p.tab_id == "tab_2"
            && matches!(&p.destination, TabTransferDestination::NewWorkspace { label } if label.as_deref() == Some("alp"))))
    );
}

#[test]
fn transfer_picker_includes_spaces_hidden_in_collapsed_worktree_groups() {
    let mut state = transfer_state();
    let mut projected = state.snapshot.as_deref().unwrap().clone();
    projected.workspaces[1].worktree = Some(ClientShellWorktree {
        key: "repo-key".into(),
        label: "repo".into(),
        is_linked_worktree: false,
    });
    projected.workspaces[2].worktree = Some(ClientShellWorktree {
        key: "repo-key".into(),
        label: "repo".into(),
        is_linked_worktree: true,
    });
    state.collapsed_groups.insert("repo-key".into());
    state.set_snapshot(Box::new(projected));
    state.compose(106, 30).unwrap();
    assert!(!state
        .hits
        .workspaces
        .iter()
        .any(|hit| hit.workspace_id == "ws_3"));
    let mut outcome = ClientShellInput::default();
    state.open_tab_transfer_picker("tab_1".into(), &mut outcome);
    assert!(state.insert_overlay_text("Beta"));
    let Some(ClientShellOverlay::TabTransfer(picker)) = &state.overlay else {
        panic!("picker")
    };
    assert_eq!(
        picker.list(state.snapshot.as_deref().unwrap()).entries[0].path,
        "ws_3"
    );
    state.submit_tab_transfer_picker(&mut outcome);
    assert!(
        matches!(&outcome.actions[..], [ClientShellAction::Endpoint { request, .. }]
        if matches!(&request.method, Method::TabTransfer(p)
            if matches!(&p.destination, TabTransferDestination::Workspace { workspace_id } if workspace_id == "ws_3")))
    );
}

#[test]
fn transfer_requires_explicit_advertisement_and_keeps_older_endpoint_usable() {
    for methods in [None, Some(vec!["tab.focus".into()])] {
        let mut state = transfer_state();
        state.set_endpoint_methods(methods);
        let mut outcome = ClientShellInput::default();
        state.open_tab_transfer_picker("tab_1".into(), &mut outcome);
        assert!(state.overlay.is_none());
        assert!(outcome.actions.is_empty());
        assert_eq!(
            state.visible_endpoint_notice.as_ref().unwrap().key.kind,
            ClientEndpointNoticeKind::Unsupported
        );
        assert!(state.endpoint_is_online(&ClientEndpointId::Local));
    }
}

#[test]
fn transfer_duplicate_submission_is_blocked_and_errors_remain_actionable() {
    let mut state = transfer_state();
    let mut outcome = ClientShellInput::default();
    state.open_tab_transfer_picker("tab_2".into(), &mut outcome);
    state.submit_tab_transfer_picker(&mut outcome);
    state.submit_tab_transfer_picker(&mut outcome);
    assert_eq!(outcome.actions.len(), 1);
    let ClientShellAction::Endpoint {
        request, boot_id, ..
    } = &outcome.actions[0]
    else {
        panic!("request")
    };
    let id = request.id.clone();
    let boot = boot_id.clone();
    let (repaint, actions) = state.handle_endpoint_result(
        &boot,
        &id,
        Err(ClientShellEndpointError {
            code: Some("endpoint_timeout".into()),
            message: "The result is uncertain.".into(),
        }),
    );
    assert!(repaint);
    assert!(actions.is_empty());
    let Some(ClientShellOverlay::TabTransfer(picker)) = &state.overlay else {
        panic!("picker")
    };
    assert!(!picker.submitting);
    assert!(picker
        .error
        .as_ref()
        .unwrap()
        .contains("Check the tab's location"));
}

#[test]
fn transfer_stale_picker_and_responses_are_rejected_after_connection_change() {
    for boot_change in [false, true] {
        let mut state = transfer_state();
        let mut outcome = ClientShellInput::default();
        state.open_tab_transfer_picker("tab_2".into(), &mut outcome);
        if boot_change {
            let mut projected = state.snapshot.as_deref().unwrap().clone();
            projected.boot_id = "replacement".into();
            // Keep the overlay to exercise the submission guard in isolation.
            state.snapshot = Some(Box::new(projected));
        } else {
            state.active_snapshot_generation = Some(99);
        }
        state.submit_tab_transfer_picker(&mut outcome);
        assert!(outcome.actions.is_empty());
        let Some(ClientShellOverlay::TabTransfer(picker)) = &state.overlay else {
            panic!("picker")
        };
        assert!(picker
            .error
            .as_ref()
            .unwrap()
            .contains("connection changed"));
    }
    let mut state = transfer_state();
    let source = state.capture_tab_transfer_source("tab_1").unwrap();
    state.active_snapshot_generation = Some(10);
    let (changed, actions) = state.complete_tab_transfer(
        &source,
        Err(ClientShellEndpointError {
            code: None,
            message: "stale response".into(),
        }),
    );
    assert!(!changed);
    assert!(actions.is_empty());
    assert!(state.endpoint_error.is_none());
}

#[test]
fn transfer_drag_goes_directly_from_tab_to_sidebar_without_focus_on_hover() {
    let mut state = transfer_state();
    let tab = state.hits.tabs[1].0;
    let target = state
        .hits
        .workspaces
        .iter()
        .find(|hit| hit.workspace_id == "ws_2")
        .unwrap()
        .rect;
    let point = (target.x + 1, target.y);
    let pressed = mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        (tab.x + 1, tab.y),
    );
    assert!(pressed.actions.is_empty());
    let dragged = mouse(&mut state, MouseEventKind::Drag(MouseButton::Left), point);
    assert!(dragged.actions.is_empty());
    assert!(
        matches!(&state.chrome_drag, Some(ClientChromeDrag::Tab { transfer_workspace_id: Some(id), .. }) if id == "ws_2")
    );
    state.compose(106, 30).unwrap();
    let released = mouse(&mut state, MouseEventKind::Up(MouseButton::Left), point);
    assert!(
        matches!(&released.actions[..], [ClientShellAction::Endpoint { request, .. }]
        if matches!(&request.method, Method::TabTransfer(p) if p.tab_id == "tab_2"
            && matches!(&p.destination, TabTransferDestination::Workspace { workspace_id } if workspace_id == "ws_2")))
    );
}

#[test]
fn transfer_drag_invalid_drops_and_escape_leave_state_unchanged() {
    for invalid in [(0, 0), (105, 29)] {
        let mut state = transfer_state();
        let tab = state.hits.tabs[0].0;
        mouse(
            &mut state,
            MouseEventKind::Down(MouseButton::Left),
            (tab.x + 1, tab.y),
        );
        mouse(&mut state, MouseEventKind::Drag(MouseButton::Left), invalid);
        let released = mouse(&mut state, MouseEventKind::Up(MouseButton::Left), invalid);
        assert!(released.actions.is_empty());
    }
    let mut state = transfer_state();
    let tab = state.hits.tabs[0].0;
    let source_workspace = state
        .hits
        .workspaces
        .iter()
        .find(|hit| hit.workspace_id == "ws_1")
        .unwrap()
        .rect;
    mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        (tab.x + 1, tab.y),
    );
    mouse(
        &mut state,
        MouseEventKind::Drag(MouseButton::Left),
        (source_workspace.x + 1, source_workspace.y),
    );
    assert!(matches!(
        &state.chrome_drag,
        Some(ClientChromeDrag::Tab {
            transfer_workspace_id: None,
            ..
        })
    ));
    let cancelled = state.handle_input_bytes(b"\x1b");
    assert!(cancelled.actions.is_empty());
    assert!(state.chrome_drag.is_none());
    let released = mouse(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        (source_workspace.x + 1, source_workspace.y),
    );
    assert!(released.actions.is_empty());
}

#[test]
fn transfer_drag_revalidates_endpoint_and_generation_at_release() {
    for stale in [false, true] {
        let mut state = transfer_state();
        let tab = state.hits.tabs[0].0;
        let source = state.capture_tab_transfer_source("tab_1").unwrap();
        let target = state
            .hits
            .workspaces
            .iter()
            .find(|hit| hit.workspace_id == "ws_2")
            .unwrap()
            .rect;
        let point = (target.x + 1, target.y);
        mouse(
            &mut state,
            MouseEventKind::Down(MouseButton::Left),
            (tab.x + 1, tab.y),
        );
        mouse(&mut state, MouseEventKind::Drag(MouseButton::Left), point);
        if stale {
            state.active_snapshot_generation = Some(4);
        } else {
            let remote = ClientEndpointId::Ssh(
                crate::client::endpoint::ProfileId::parse("0123456789abcdef0123456789abcdef")
                    .unwrap(),
            );
            state
                .hits
                .workspaces
                .iter_mut()
                .find(|hit| hit.workspace_id == "ws_2")
                .unwrap()
                .endpoint_id = remote;
        }
        assert!(state.tab_transfer_drop_target(&source, point).is_none());
        let released = mouse(&mut state, MouseEventKind::Up(MouseButton::Left), point);
        assert!(released.actions.is_empty());
    }
}

#[test]
fn transfer_keybinding_accepts_both_configured_prefixes() {
    let config: Config = toml::from_str("[keys]\nprefix = [\"ctrl+b\", \"ctrl+a\"]").unwrap();
    for prefix in [b'\x02', b'\x01'] {
        let mut state = transfer_state();
        state.config = ClientShellConfig::from_config(&config);
        let outcome = state.handle_input_bytes(&[prefix, b'M']);
        assert!(outcome.actions.is_empty());
        assert!(matches!(
            &state.overlay,
            Some(ClientShellOverlay::TabTransfer(_))
        ));
    }
}

#[test]
fn transfer_picker_stays_stale_when_same_boot_reconnect_precedes_new_snapshot() {
    let mut state = transfer_state();
    let mut outcome = ClientShellInput::default();
    state.open_tab_transfer_picker("tab_2".into(), &mut outcome);
    state.set_endpoint_status(&ClientEndpointId::Local, ClientEndpointStatus::Reconnecting);
    state.set_endpoint_status(&ClientEndpointId::Local, ClientEndpointStatus::Online);
    state.submit_tab_transfer_picker(&mut outcome);
    assert!(outcome.actions.is_empty());
    let Some(ClientShellOverlay::TabTransfer(picker)) = &state.overlay else {
        panic!("picker")
    };
    assert!(picker
        .error
        .as_ref()
        .unwrap()
        .contains("connection changed"));
}

#[test]
fn transfer_drag_accepts_visible_worktree_child_rows_and_highlights_them() {
    let mut state = transfer_state();
    let mut projected = state.snapshot.as_deref().unwrap().clone();
    projected.workspaces[1].worktree = Some(ClientShellWorktree {
        key: "repo-key".into(),
        label: "repo".into(),
        is_linked_worktree: false,
    });
    projected.workspaces[2].worktree = Some(ClientShellWorktree {
        key: "repo-key".into(),
        label: "repo".into(),
        is_linked_worktree: true,
    });
    state.set_snapshot(Box::new(projected));
    state.compose(106, 30).unwrap();
    let tab = state.hits.tabs[1].0;
    let child = state
        .hits
        .workspaces
        .iter()
        .find(|hit| hit.workspace_id == "ws_3")
        .unwrap();
    assert!(child.indented);
    let target = child.rect;
    let point = (target.x + 1, target.y);
    mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        (tab.x + 1, tab.y),
    );
    mouse(&mut state, MouseEventKind::Drag(MouseButton::Left), point);
    let composed = state.compose(106, 30).unwrap();
    assert_eq!(
        composed.cells[(point.1 * composed.width + point.0) as usize].bg,
        crate::protocol::color_to_u32(state.config.palette.active_row_bg)
    );
    let released = mouse(&mut state, MouseEventKind::Up(MouseButton::Left), point);
    assert!(
        matches!(&released.actions[..], [ClientShellAction::Endpoint { request, .. }]
        if matches!(&request.method, Method::TabTransfer(p)
            if matches!(&p.destination, TabTransferDestination::Workspace { workspace_id } if workspace_id == "ws_3")))
    );
}

#[test]
fn transfer_help_lists_live_binding_and_new_space_cancel_has_no_requests() {
    let mut state = transfer_state();
    let groups = crate::input::keybind_help_groups(
        &state.config.keybinds.keybinds,
        &state.config.keybinds.prefix,
    );
    assert!(groups
        .iter()
        .flat_map(|(_, rows)| rows)
        .any(|(_, label)| label == "move tab to space"));
    let mut outcome = ClientShellInput::default();
    state.open_tab_transfer_picker("tab_2".into(), &mut outcome);
    state.insert_overlay_text("unique-space-name");
    state.submit_tab_transfer_picker(&mut outcome);
    assert!(matches!(state.overlay, Some(ClientShellOverlay::Rename(_))));
    let cancelled = state.handle_input_bytes(b"\x1b");
    assert!(cancelled.actions.is_empty());
    assert!(outcome.actions.is_empty());
    assert!(state.overlay.is_none());
}
