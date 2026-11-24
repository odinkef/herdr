use super::*;
use crate::api::schema::{Method, ResponseResult, WorkspaceTodo};
use crate::client::shell::tests::{snapshot, surface};
use crate::input::TerminalKey;

fn shell() -> ClientShellState {
    shell_with_keys("")
}

fn shell_with_keys(bindings: &str) -> ClientShellState {
    let config: Config = toml::from_str(&format!(
        "[keys]\nprefix = ['ctrl+a', 'ctrl+b']\n{bindings}"
    ))
    .unwrap();
    let mut shell = ClientShellState::new(ClientShellConfig::from_config(&config));
    shell.set_endpoint_methods(Some(
        [
            "workspace.todos.list",
            "workspace.todos.add",
            "workspace.todos.set_done",
            "workspace.todos.delete",
            "pane.focus",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
    ));
    let mut snap = snapshot();
    let mut other = snap.panes[0].clone();
    other.pane_id = "pane_2".into();
    snap.panes[0].focused = false;
    snap.focused_pane_id = Some(other.pane_id.clone());
    snap.panes.push(other);
    shell.set_snapshot(Box::new(snap));
    let mut split = surface();
    split.frame = FrameData::from_ratatui_buffer_with_hyperlinks(
        &Buffer::empty(Rect::new(0, 0, 24, 12)),
        None,
        &[],
    );
    let first = &mut split.panes[0];
    first.rect.width = 12;
    first.rect.height = 12;
    first.inner_rect = first.rect;
    first.focused = false;
    let mut second = first.clone();
    second.rect.x = 12;
    second.inner_rect = second.rect;
    second.pane_id = "pane_2".into();
    second.focused = true;
    split.panes.push(second);
    shell.set_pane_surface(split);
    shell.complete_todos(Ok(ResponseResult::WorkspaceTodos {
        workspace_id: "ws_1".into(),
        revision: 1,
        todos: (1..=20)
            .map(|id| WorkspaceTodo {
                id,
                label: format!("task {id}"),
                done: false,
            })
            .collect(),
    }));
    shell.compose(100, 30).unwrap();
    shell
}

fn todo_area(state: &ClientShellState) -> Rect {
    state.todo_sidebar_area(state.layout(100, 30).sidebar)
}

#[test]
fn todo_selection_line_follows_arrows_and_scrolls_with_the_task() {
    for collapsed in [false, true] {
        for palette in [Palette::catppuccin(), Palette::terminal()] {
            let mut state = shell();
            state.config.palette = palette;
            state.sidebar_collapsed = collapsed;
            state.todo_data.items[1].done = true;
            let steps = std::iter::once((b"\x02t".as_slice(), 1))
                .chain((2..=14).map(|id| (b"\x1b[B".as_slice(), id)))
                .chain((1..14).rev().map(|id| (b"\x1b[A".as_slice(), id)));
            for (keys, id) in steps {
                state.handle_input_bytes(keys);
                let buffer = state.compose(100, 30).unwrap().to_ratatui_buffer().unwrap();
                let palette = &state.config.palette;
                assert!(state.hits.todos.rows.iter().any(|(_, task)| *task == id));
                for (row, task) in &state.hits.todos.rows {
                    let selected = *task == id;
                    let expected_bg = if selected {
                        palette.active_row_bg
                    } else {
                        palette.sidebar_bg
                    };
                    for x in row.x..row.right() {
                        assert_eq!(buffer[(x, row.y)].bg, expected_bg);
                    }
                    assert_eq!(buffer[(row.x, row.y)].symbol(), " ");
                    assert_eq!(
                        buffer[(row.x + 5, row.y)].modifier.contains(Modifier::BOLD),
                        selected
                    );
                    assert_eq!(
                        buffer[(row.x + 2, row.y)].symbol(),
                        if *task == 2 { "x" } else { " " }
                    );
                }
            }
        }
    }
}

#[test]
fn todo_list_resolves_configured_resize_keys_and_repeats_without_forwarding_input() {
    let mut state = shell_with_keys(
        "resize_pane_up = ['alt+shift+up', 'prefix+f12']\n\
         resize_pane_down = 'alt+shift+down'\nresize_pane_left = 'alt+shift+left'\n\
         resize_pane_right = 'alt+shift+right'",
    );
    state.handle_input_bytes(b"\x01t");
    let initial = todo_area(&state);
    let key = TerminalKey::new(KeyCode::Up, KeyModifiers::ALT | KeyModifiers::SHIFT);
    let repeated = state.handle_raw_events(vec![RawInputEvent::Key(key.with_repeat_count(3))]);
    assert!(repeated.actions.is_empty() && repeated.requests.is_empty());
    assert!(todo_area(&state).height >= initial.height + 3);

    let height = todo_area(&state).height;
    state.handle_input_bytes(b"\x1b[1;4B");
    assert!(todo_area(&state).height < height);
    state.handle_input_bytes(b"\x02\x1b[24~");
    assert_eq!(todo_area(&state).height, height);
    assert_eq!(state.mode, ClientShellMode::Todos);
    let narrower = state.handle_input_bytes(b"\x1b[1;4D");
    assert!(narrower.resize && narrower.actions.is_empty() && narrower.requests.is_empty());
    assert!(todo_area(&state).width < initial.width);
    let wider = state.handle_input_bytes(b"\x1b[1;4C");
    assert!(wider.resize && wider.actions.is_empty() && wider.requests.is_empty());
    assert_eq!(todo_area(&state).width, initial.width);
    assert_eq!(state.todo_data.selected, Some(1));
    assert_eq!(state.mode, ClientShellMode::Todos);
}

#[test]
fn todo_resize_mode_returns_to_list_and_editor_keeps_text_input() {
    for (prefix, exit) in [(1, b"\x1b".as_slice()), (2, b"\r"), (1, b"R")] {
        let mut state = shell_with_keys("resize_mode = 'prefix+shift+r'");
        state.handle_input_bytes(&[prefix, b't']);
        let entered = state.handle_input_bytes(&[prefix, b'R']);
        assert_eq!(state.mode, ClientShellMode::TodoResize);
        assert!(entered.actions.is_empty() && entered.requests.is_empty());
        let initial = todo_area(&state);
        let enlarge = state.handle_input_bytes(b"k\x1b[A");
        assert!(enlarge.actions.is_empty() && enlarge.requests.is_empty());
        assert!(todo_area(&state).height >= initial.height + 2);
        state.handle_input_bytes(b"j\x1b[B");
        assert_eq!(todo_area(&state).height, initial.height);
        state.handle_input_bytes(b"h");
        assert!(todo_area(&state).width < initial.width);
        state.handle_input_bytes(b"l");
        assert_eq!(todo_area(&state).width, initial.width);
        let ignored = state.handle_input_bytes(b"n ");
        assert!(ignored.actions.is_empty() && ignored.requests.is_empty());
        assert_eq!(state.mode, ClientShellMode::TodoResize);
        state.handle_input_bytes(exit);
        assert_eq!(state.mode, ClientShellMode::Todos);
        let canceled = state.handle_input_bytes(&[prefix, prefix, prefix, b'x']);
        assert!(canceled.actions.is_empty() && canceled.requests.is_empty());
        assert_eq!(state.mode, ClientShellMode::Todos);
        state.handle_input_bytes(b"nhjklR");
        assert_eq!(state.mode, ClientShellMode::TodoEditor);
        assert_eq!(state.todo_ui.editor.as_str(), "hjklR");
        assert_eq!(todo_area(&state), initial);
        state.handle_input_bytes(b"\x1b");
        state.handle_input_bytes(b"\x1b");
        assert!(matches!(
            state.handle_input_bytes(b"x").requests.as_slice(),
            [ClientMessage::ClientShellPaneInput {pane_id, ..}] if pane_id == "pane_2"
        ));
    }
}

fn method(out: &ClientShellInput) -> &Method {
    let [ClientShellAction::Endpoint { request, .. }] = &out.actions[..] else {
        panic!("expected one endpoint action: {:?}", out.actions);
    };
    &request.method
}

fn mouse(shell: &mut ClientShellState, kind: MouseEventKind, x: u16, y: u16) -> ClientShellInput {
    shell.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind,
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    })])
}

#[test]
fn todo_prefix_editor_and_held_delete_return_typing_to_the_original_split() {
    for prefix in [1, 2] {
        let mut state = shell();
        let enter = state.handle_input_bytes(&[prefix, b't']);
        assert!(
            matches!(method(&enter), Method::WorkspaceTodosList(p) if p.workspace_id == "ws_1")
        );
        assert_eq!(state.mode, ClientShellMode::Todos);
        state.handle_input_bytes(b"\x1b[B");
        assert_eq!(state.todo_data.selected, Some(2));
        let check = state.handle_input_bytes(b" ");
        assert!(
            matches!(method(&check), Method::WorkspaceTodosSetDone(p) if p.todo_id == 2 && p.done)
        );
        let delete = TerminalKey::new(KeyCode::Delete, KeyModifiers::NONE);
        let remove = state.handle_raw_events(vec![RawInputEvent::Key(
            delete.clone().with_repeat_count(8),
        )]);
        assert!(matches!(method(&remove), Method::WorkspaceTodosDelete(p) if p.todo_id == 2));
        // A response can select the neighbor while the key is still physically held.
        state.todo_data.selected = Some(3);
        let repeat = state.handle_raw_events(vec![RawInputEvent::Key(
            delete.with_kind(KeyEventKind::Repeat),
        )]);
        assert!(repeat.actions.is_empty() && repeat.requests.is_empty());

        state.handle_input_bytes(b"n");
        state.handle_raw_events(vec![RawInputEvent::Paste("first\nsecond".into())]);
        let submitted = state.handle_input_bytes(b"\r");
        assert!(
            matches!(method(&submitted), Method::WorkspaceTodosAdd(p) if p.label == "first second")
        );
        let canceled = state.handle_input_bytes(b"nunused\x1b");
        assert!(canceled.requests.is_empty() && canceled.actions.is_empty());
        assert_eq!(state.mode, ClientShellMode::Todos);
        let returned = state.handle_input_bytes(b"\x1b");
        assert!(returned.actions.is_empty());
        assert_eq!(state.mode, ClientShellMode::Terminal);
        assert!(
            matches!(state.handle_input_bytes(b"h").requests.as_slice(), [ClientMessage::ClientShellPaneInput{pane_id, ..}] if pane_id == "pane_2")
        );
    }
}

#[test]
fn todo_mouse_and_overlay_share_hit_targets_and_preserve_the_workspace_divider() {
    for (size, collapsed, expected_height) in [
        ((100, 20), false, 5),
        ((100, 80), false, 10),
        ((100, 15), false, 4),
        ((100, 30), true, 0),
        ((100, 12), false, 0),
        ((30, 20), false, 0),
    ] {
        let mut state = shell();
        state.sidebar_collapsed = collapsed;
        state.compose(size.0, size.1).unwrap();
        if expected_height == 0 {
            assert!(state.hits.todos.area.is_empty());
            state.handle_input_bytes(b"\x02t");
            state.compose(size.0, size.1).unwrap();
        } else {
            assert!(state.hits.sidebar_section_divider.bottom() <= state.hits.todos.area.y);
        }
        assert_eq!(state.hits.todos.sidebar_height, expected_height);
        let (row, id) = state.hits.todos.rows[0];
        let select = mouse(
            &mut state,
            MouseEventKind::Down(MouseButton::Left),
            row.x + 6,
            row.y,
        );
        assert!(select.actions.iter().all(|a| matches!(a, ClientShellAction::Endpoint{request,..} if matches!(request.method, Method::WorkspaceTodosList(_)))));
        assert_eq!(state.todo_data.selected, Some(id));
        let check = mouse(
            &mut state,
            MouseEventKind::Down(MouseButton::Left),
            row.x + 2,
            row.y,
        );
        assert!(matches!(method(&check), Method::WorkspaceTodosSetDone(p) if p.todo_id == id));
        mouse(&mut state, MouseEventKind::ScrollDown, row.x + 2, row.y);
        state.compose(size.0, size.1).unwrap();
        assert!(state.hits.todos.rows[0].1 > id);
        let plus = state.hits.todos.add;
        mouse(
            &mut state,
            MouseEventKind::Down(MouseButton::Left),
            plus.x + 1,
            plus.y,
        );
        assert_eq!(state.mode, ClientShellMode::TodoEditor);
        let frame = state.compose(size.0, size.1).unwrap();
        let cursor = frame.cursor.as_ref().unwrap();
        assert!(contains(state.hits.todos.area, (cursor.x, cursor.y)));
        let pane = state.hits.panes[0].inner_rect;
        let visible_row = (pane.y..pane.bottom().min(size.1))
            .find(|&y| !contains(state.hits.todos.area, (pane.x, y)))
            .expect("visible terminal area");
        let focus = mouse(
            &mut state,
            MouseEventKind::Down(MouseButton::Left),
            pane.x,
            visible_row,
        );
        assert!(matches!(method(&focus), Method::PaneFocus(p) if p.pane_id == "pane_1"));
        assert_eq!(state.mode, ClientShellMode::Terminal);
        assert!(
            matches!(state.handle_input_bytes(b"h").requests.as_slice(), [ClientMessage::ClientShellPaneInput{pane_id, ..}] if pane_id == "pane_1")
        );
    }
}

#[test]
fn todo_escape_restores_the_saved_pane_if_another_client_changed_focus() {
    let mut state = shell();
    state.handle_input_bytes(b"\x02t");
    let mut changed = state.snapshot.as_ref().unwrap().as_ref().clone();
    changed.revision += 1;
    changed.focused_pane_id = Some("pane_1".into());
    changed.panes[0].focused = true;
    changed.panes[1].focused = false;
    state.set_snapshot(Box::new(changed));
    let escaped = state.handle_input_bytes(b"\x1b");
    assert!(matches!(method(&escaped), Method::PaneFocus(p) if p.pane_id == "pane_2"));
    assert!(
        matches!(state.handle_input_bytes(b"h").requests.as_slice(), [ClientMessage::ClientShellPaneInput{pane_id, ..}] if pane_id == "pane_2")
    );
}

#[test]
fn todo_focus_falls_back_when_the_pane_or_workspace_disappears() {
    let mut state = shell();
    state.handle_input_bytes(b"\x02t");
    let mut snapshot = state.snapshot.as_deref().unwrap().clone();
    snapshot.panes.retain(|pane| pane.pane_id == "pane_1");
    snapshot.panes[0].focused = true;
    snapshot.focused_pane_id = Some("pane_1".into());
    snapshot.revision += 1;
    state.set_snapshot(Box::new(snapshot.clone()));
    let escaped = state.handle_input_bytes(b"\x1b");
    assert!(escaped.actions.is_empty());
    let typed = state.handle_input_bytes(b"h");
    assert!(
        matches!(typed.requests.as_slice(), [ClientMessage::ClientShellPaneInput { pane_id, .. }] if pane_id == "pane_1")
    );

    state.handle_input_bytes(b"\x02tnold workspace");
    assert_eq!(state.mode, ClientShellMode::TodoEditor);
    snapshot.revision += 1;
    snapshot.focused_workspace_id = None;
    snapshot.focused_tab_id = None;
    snapshot.focused_pane_id = None;
    snapshot.workspaces.clear();
    snapshot.tabs.clear();
    snapshot.panes.clear();
    state.set_snapshot(Box::new(snapshot));
    let result = state.handle_input_bytes(b"\r");
    assert_eq!(state.mode, ClientShellMode::Terminal);
    assert!(result.actions.is_empty() && result.requests.is_empty());
}

#[test]
fn todo_dividers_resize_without_stealing_focus_and_save_on_release() {
    let mut state = shell();
    let path = std::env::temp_dir().join(format!("herdr-todo-resize-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&path);
    state.config.preferences_path = Some(path.clone());
    state.handle_input_bytes(b"\x02tnkeep typing");
    let selected = state.todo_data.selected;
    let panel = state.hits.todos.area;
    assert_eq!(panel.height, 7);
    mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        panel.x + 3,
        panel.y,
    );
    let dragged = mouse(
        &mut state,
        MouseEventKind::Drag(MouseButton::Left),
        panel.x + 3,
        3,
    );
    assert!(
        dragged.repaint
            && !dragged.resize
            && dragged.actions.is_empty()
            && dragged.requests.is_empty()
    );
    assert!(!path.exists());
    state.compose(100, 30).unwrap();
    assert_eq!(state.hits.todos.area.height, 20);
    assert!(!state.hits.agent_body.is_empty() && !state.hits.workspace_body.is_empty());
    assert!(state.hits.sidebar_section_divider.y < state.hits.todos.area.y);
    assert_eq!(state.todo_data.selected, selected);
    mouse(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        panel.x + 3,
        3,
    );
    assert!(path.exists());
    assert_eq!(state.mode, ClientShellMode::TodoEditor);
    assert_eq!(state.todo_ui.editor.as_str(), "keep typing");

    // A shorter window clamps the visible size. Growing the window restores it.
    state.compose(100, 16).unwrap();
    assert_eq!(state.hits.todos.area.height, 6);
    state.compose(100, 30).unwrap();
    assert_eq!(state.hits.todos.area.height, 20);

    let divider = state.hits.sidebar_divider;
    let width = state.sidebar_width + 2;
    mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        divider.x,
        24,
    );
    let drag_width = mouse(
        &mut state,
        MouseEventKind::Drag(MouseButton::Left),
        width - 1,
        24,
    );
    assert!(drag_width.resize && drag_width.repaint);
    assert_eq!(state.mode, ClientShellMode::TodoEditor);
    state.compose(100, 30).unwrap();
    mouse(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        width - 1,
        24,
    );
    let restored = ClientShellState::new(state.config.with_preferences_path(path.clone()));
    assert_eq!(restored.sidebar_width, width);
    assert_eq!(restored.todo_size.todo_height, Some(20));
    std::fs::remove_file(path).unwrap();
}

#[test]
fn todo_overlay_corner_and_border_resize_from_measured_size_without_resizing_panes() {
    use crate::api::schema::PaneDirection;
    let mut state = shell();
    let path = std::env::temp_dir().join(format!(
        "herdr-todo-overlay-drag-{}.json",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    state.config.preferences_path = Some(path.clone());
    state.sidebar_collapsed = true;
    state.handle_input_bytes(b"\x02t");
    state.compose(100, 30).unwrap();
    let initial = state.hits.todos.area;
    assert_eq!((initial.width, initial.height), (54, 12));
    mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        initial.x,
        initial.y,
    );
    let drag = mouse(
        &mut state,
        MouseEventKind::Drag(MouseButton::Left),
        initial.x - 3,
        initial.y - 2,
    );
    assert!(drag.repaint && !drag.resize && drag.actions.is_empty() && drag.requests.is_empty());
    mouse(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        initial.x - 3,
        initial.y - 2,
    );
    state.compose(100, 30).unwrap();
    let larger = state.hits.todos.area;
    assert_eq!(larger, Rect::new(initial.x - 3, initial.y - 2, 60, 16));

    // The width is clipped in the smaller terminal. Dragging only the bottom edge
    // commits a new height while retaining the width for a larger terminal.
    state.compose(30, 20).unwrap();
    let compact = state.hits.todos.area;
    assert_eq!(compact.width, 26);
    let x = compact.x + compact.width / 2;
    mouse(
        &mut state,
        MouseEventKind::Down(MouseButton::Left),
        x,
        compact.bottom() - 1,
    );
    mouse(
        &mut state,
        MouseEventKind::Up(MouseButton::Left),
        x,
        compact.bottom() - 3,
    );
    state.compose(30, 20).unwrap();
    assert_eq!(state.hits.todos.area.height, 12);
    let selected = state.todo_data.selected;
    let mut outcome = ClientShellInput::default();
    state.resize_todos(PaneDirection::Right, &mut outcome);
    assert!(!outcome.repaint);
    state.resize_todos(PaneDirection::Down, &mut outcome);
    assert!(outcome.repaint && !outcome.resize);
    state.compose(100, 30).unwrap();
    assert_eq!(
        (state.hits.todos.area.width, state.hits.todos.area.height),
        (60, 11)
    );
    assert_eq!(state.mode, ClientShellMode::Todos);
    assert_eq!(state.todo_data.selected, selected);
    assert!(state.chrome_drag.is_none());
    let restored = ClientShellState::new(state.config.with_preferences_path(path.clone()));
    assert_eq!(restored.todo_size.todo_overlay_width, Some(60));
    assert_eq!(restored.todo_size.todo_overlay_height, Some(11));
    assert_eq!(restored.todo_size.todo_height, None);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn manual_todo_height_respects_sidebar_origin_and_unset_overlay_keeps_cramped_geometry() {
    let mut state = shell();
    state.todo_size.todo_height = Some(300);
    assert_eq!(
        state.todo_sidebar_area(Rect::new(5, 7, 22, 34)),
        Rect::new(5, 17, 20, 24)
    );
    assert!(state.todo_sidebar_area(Rect::new(5, 7, 22, 13)).is_empty());
    for (width, height) in [(4, 40), (80, 5), (7, 7), (3, 2), (10, 8), (80, 40)] {
        let screen = Rect::new(0, 0, width, height);
        assert_eq!(
            state.todo_overlay_area(screen),
            crate::ui::centered_popup_rect(screen, 54, 12).unwrap_or(screen)
        );
    }
}
