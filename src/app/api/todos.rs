use crate::api::schema::ResponseResult;
use crate::app::App;
use crate::workspace::WorkspaceTodoList;

use super::responses::{encode_error, encode_success};
use super::workspaces::workspace_not_found;

impl App {
    pub(super) fn handle_workspace_todos(
        &mut self,
        id: String,
        workspace_id: String,
        edit: impl FnOnce(&mut WorkspaceTodoList) -> Result<(), &'static str>,
    ) -> String {
        // A positional workspace alias could point at another workspace on a deletion retry.
        let Some(workspace) = self
            .state
            .workspaces
            .iter_mut()
            .find(|ws| ws.id == workspace_id)
        else {
            return workspace_not_found(id, &workspace_id);
        };
        let todos = &mut workspace.todos;
        let previous_revision = todos.revision;
        if let Err(message) = edit(todos) {
            return encode_error(id, "workspace_todos_failed", message);
        }
        let changed = todos.revision != previous_revision;
        let result = ResponseResult::WorkspaceTodos {
            workspace_id,
            revision: todos.revision,
            todos: todos.items.clone(),
        };
        if changed {
            self.schedule_session_save();
        }
        encode_success(id, result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn call(app: &mut App, method: &str, params: Value) -> Value {
        let request =
            serde_json::from_value(json!({"id":"todos", "method":method, "params":params}))
                .unwrap();
        serde_json::from_str(&app.handle_api_request(request)).unwrap()
    }

    #[test]
    fn workspace_todos_address_stable_identity_and_accept_delete_retries() {
        let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            rx,
            crate::api::EventHub::default(),
        );
        app.state = crate::app::AppState::test_with_adversarial_identity_state();
        app.state
            .workspaces
            .push(crate::workspace::Workspace::test_new("linked"));
        app.state.ensure_test_terminals();
        let first = app.public_workspace_id(0);
        let linked = app.public_workspace_id(1);
        for ws in [&first, &linked] {
            let added = call(
                &mut app,
                "workspace.todos.add",
                json!({"workspace_id":ws,"label":"same"}),
            );
            assert_eq!(
                added["result"],
                json!({
                    "type":"workspace_todos", "workspace_id":ws, "revision":1,
                    "todos":[{"id":1,"label":"same","done":false}]
                })
            );
        }
        call(
            &mut app,
            "workspace.rename",
            json!({"workspace_id":first,"label":"renamed"}),
        );
        call(
            &mut app,
            "workspace.move",
            json!({"workspace_id":first,"insert_index":2}),
        );
        assert_eq!(app.public_workspace_id(1), first);
        app.state.workspaces[1].switch_tab(1);
        let done = json!({"workspace_id":first,"todo_id":1,"done":true});
        for _ in 0..2 {
            let updated = call(&mut app, "workspace.todos.set_done", done.clone());
            assert_eq!(updated["result"]["revision"], 2);
            assert_eq!(updated["result"]["todos"][0]["done"], true);
        }
        for _ in 0..2 {
            let deleted = call(
                &mut app,
                "workspace.todos.delete",
                json!({"workspace_id":first,"todo_id":1}),
            );
            assert_eq!(deleted["result"]["revision"], 3);
            assert_eq!(deleted["result"]["todos"], json!([]));
        }
        let listed = call(
            &mut app,
            "workspace.todos.list",
            json!({"workspace_id":linked}),
        );
        assert_eq!(
            listed["result"]["todos"],
            json!([{"id":1,"label":"same","done":false}])
        );
        for label in ["  ", "two\nlines", "escape\u{1b}"] {
            let invalid = call(
                &mut app,
                "workspace.todos.add",
                json!({"workspace_id":first,"label":label}),
            );
            assert!(invalid.get("error").is_some());
        }
        let missing = call(&mut app, "workspace.todos.set_done", done);
        assert!(missing.get("error").is_some());
        let positional = call(
            &mut app,
            "workspace.todos.delete",
            json!({"workspace_id":"1","todo_id":1}),
        );
        assert_eq!(positional["error"]["code"], "workspace_not_found");
        app.state.assert_invariants_for_test();
    }
}
