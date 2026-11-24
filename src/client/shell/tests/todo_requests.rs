use super::*;
use crate::api::schema::{Method, ResponseResult, WorkspaceTodo};

impl ClientShellState {
    pub(crate) fn populate_todos_for_profile(&mut self, workspace_id: &str) {
        self.set_endpoint_methods(Some(vec!["workspace.todos.list".into()]));
        let ids = (1..=20).collect::<Vec<_>>();
        assert!(self.complete_todos(Ok(response(workspace_id, 1, &ids))));
    }
}

fn todo_client() -> ClientShellState {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_endpoint_methods(Some(
        ["list", "add", "set_done", "delete"]
            .map(|s| format!("workspace.todos.{s}"))
            .to_vec(),
    ));
    state.set_snapshot(Box::new(snapshot()));
    state
}

fn request(outcome: &ClientShellInput) -> &crate::api::schema::Request {
    let [ClientShellAction::Endpoint { request, .. }] = outcome.actions.as_slice() else {
        panic!("expected one endpoint request");
    };
    request
}

fn list(state: &mut ClientShellState, workspace_id: &str) -> String {
    let mut outcome = ClientShellInput::default();
    state.refresh_todos_if_needed(&mut outcome);
    let request = request(&outcome);
    assert!(
        matches!(&request.method, Method::WorkspaceTodosList(params) if params.workspace_id == workspace_id)
    );
    request.id.clone()
}

fn response(workspace: &str, revision: u64, ids: &[u64]) -> ResponseResult {
    ResponseResult::WorkspaceTodos {
        workspace_id: workspace.into(),
        revision,
        todos: ids
            .iter()
            .map(|id| WorkspaceTodo {
                id: *id,
                label: "same label".into(),
                done: false,
            })
            .collect(),
    }
}

#[test]
fn todo_workspace_switch_and_reconnect_retire_stale_replies() {
    let mut state = todo_client();
    let original = list(&mut state, "ws_1");
    let mut next = snapshot();
    next.revision += 1;
    next.focused_workspace_id = Some("ws_2".into());
    state.set_snapshot(Box::new(next.clone()));
    let second = list(&mut state, "ws_2");
    state.handle_endpoint_result("boot-1", &second, Ok(response("ws_2", 2, &[1])));
    let mut delete = ClientShellInput::default();
    assert!(state.delete_todo(1, &mut delete));
    assert!(
        matches!(&request(&delete).method, Method::WorkspaceTodosDelete(p) if p.workspace_id == "ws_2" && p.todo_id == 1)
    );
    next.revision += 1;
    next.focused_workspace_id = Some("ws_1".into());
    state.set_snapshot(Box::new(next.clone()));
    assert!(state.todo_data.items.is_empty());
    assert!(
        !state
            .handle_endpoint_result("boot-1", &original, Ok(response("ws_1", 90, &[99])))
            .0
    );
    assert!(
        !state
            .handle_endpoint_result("boot-1", &request(&delete).id, Ok(response("ws_2", 3, &[])))
            .0
    );
    let latest = list(&mut state, "ws_1");
    assert!(
        state
            .handle_endpoint_result("boot-1", &latest, Ok(response("ws_1", 2, &[1])))
            .0
    );
    assert_eq!(state.todo_data.selected, Some(1));
    let mut refresh = ClientShellInput::default();
    state.refresh_todos_if_needed(&mut refresh);
    assert!(refresh.actions.is_empty());
    state.refresh_todos(&mut refresh);
    state.mark_endpoint_disconnected(&ClientEndpointId::Local);
    state.set_endpoint_status(&ClientEndpointId::Local, ClientEndpointStatus::Online);
    state.set_endpoint_snapshot_for_generation(&ClientEndpointId::Local, 2, Box::new(next));
    assert!(state.todo_data.items.is_empty());
    assert!(
        !state
            .handle_endpoint_result(
                "boot-1",
                &request(&refresh).id,
                Ok(response("ws_1", 3, &[5]))
            )
            .0
    );
    list(&mut state, "ws_1");
}

#[test]
fn todo_selection_moves_only_after_confirmed_deletion_and_survives_stale_reads() {
    let mut state = todo_client();
    let id = list(&mut state, "ws_1");
    state.handle_endpoint_result("boot-1", &id, Ok(response("ws_1", 5, &[1, 2, 4])));
    let mut refresh = ClientShellInput::default();
    state.refresh_todos(&mut refresh);
    for (deleted, ids, selected) in [
        (1, vec![2, 4], Some(2)),
        (4, vec![2], Some(2)),
        (2, vec![], None),
    ] {
        state.todo_data.selected = Some(deleted);
        let mut out = ClientShellInput::default();
        assert!(state.delete_todo(deleted, &mut out));
        assert!(
            matches!(&request(&out).method, Method::WorkspaceTodosDelete(p) if p.todo_id == deleted && p.workspace_id == "ws_1")
        );
        assert!(state.todo_data.items.iter().any(|item| item.id == deleted));
        state.handle_endpoint_result(
            "boot-1",
            &request(&out).id,
            Ok(response("ws_1", 9 - ids.len() as u64, &ids)),
        );
        assert_eq!(state.todo_data.selected, selected);
    }
    assert!(
        !state
            .handle_endpoint_result(
                "boot-1",
                &request(&refresh).id,
                Ok(response("ws_1", 5, &[1, 2, 4]))
            )
            .0
    );
    assert!(state.todo_data.items.is_empty());
}

#[test]
fn unavailable_todo_methods_only_notify_for_explicit_actions() {
    let mut state = todo_client();
    state.set_endpoint_methods(Some(Vec::new()));
    let mut out = ClientShellInput::default();
    state.refresh_todos_if_needed(&mut out);
    assert!(state.visible_endpoint_notice.is_none());
    assert!(!state.refresh_todos(&mut out));
    assert!(out.actions.is_empty());
    assert_eq!(
        state.visible_endpoint_notice.as_ref().unwrap().title,
        "Action unavailable"
    );
    state.set_endpoint_methods(Some(vec!["workspace.todos.list".into()]));
    assert!(state.refresh_todos(&mut out));
    assert!(!state.add_todo("new".into(), &mut out));
    assert_eq!(out.actions.len(), 1);
    assert_eq!(
        state.visible_endpoint_notice.as_ref().unwrap().key.code,
        "workspace.todos.add"
    );
}

#[test]
fn quick_toggles_queue_distinct_settings_without_changing_the_confirmed_checkbox() {
    let mut state = todo_client();
    let id = list(&mut state, "ws_1");
    state.handle_endpoint_result("boot-1", &id, Ok(response("ws_1", 1, &[1])));
    for expected in [true, false, true] {
        let mut out = ClientShellInput::default();
        assert!(state.toggle_todo_done(1, &mut out));
        assert!(
            matches!(&request(&out).method, Method::WorkspaceTodosSetDone(p) if p.todo_id == 1 && p.done == expected)
        );
        assert!(!state.todo_data.items[0].done);
    }
}
