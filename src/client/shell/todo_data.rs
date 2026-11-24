use super::*;
use crate::api::schema::{
    Method, ResponseResult, WorkspaceTarget, WorkspaceTodo, WorkspaceTodoAddParams,
    WorkspaceTodoSetDoneParams, WorkspaceTodoTarget,
};

const LIST_METHOD: &str = "workspace.todos.list";

#[derive(Default)]
pub(super) struct ClientTodos {
    pub(super) items: Vec<WorkspaceTodo>,
    pub(super) selected: Option<u64>,
    revision: Option<u64>,
    available: bool,
    refresh_needed: bool,
}

impl ClientShellState {
    pub(super) fn todos_available(&self) -> bool {
        self.todo_data.available
    }

    pub(super) fn update_todo_availability(&mut self) {
        let available = self
            .endpoints
            .iter()
            .find(|endpoint| endpoint.endpoint_id == self.active_endpoint_id)
            .and_then(|endpoint| endpoint.methods.as_ref())
            .is_some_and(|methods| methods.contains(LIST_METHOD));
        self.todo_data.refresh_needed =
            available && (self.todo_data.refresh_needed || !self.todo_data.available);
        self.todo_data.available = available;
    }

    pub(super) fn reset_todos(&mut self) {
        // Returning to the same workspace (or a new connection with the same boot ID)
        // starts a new view. Its requests have distinct IDs from any retired view.
        self.pending_requests.retain(|_, request| {
            !matches!(
                request.kind,
                PendingEndpointKind::WorkspaceTodos | PendingEndpointKind::SetTodoDone { .. }
            )
        });
        self.todo_data = ClientTodos::default();
        self.update_todo_availability();
        self.reset_todo_ui();
    }

    pub(crate) fn refresh_todos_if_needed(&mut self, outcome: &mut ClientShellInput) {
        if self.todo_data.refresh_needed && self.endpoint_is_online(&self.active_endpoint_id) {
            self.refresh_todos(outcome);
        }
    }

    fn todo_workspace_id(&self) -> Option<String> {
        self.snapshot.as_ref()?.focused_workspace_id.clone()
    }

    pub(super) fn refresh_todos(&mut self, outcome: &mut ClientShellInput) -> bool {
        self.todo_data.refresh_needed = false;
        if !self.todos_available() {
            outcome.repaint |= self.unsupported_endpoint_action(LIST_METHOD);
            return false;
        }
        let Some(workspace_id) = self.todo_workspace_id() else {
            return false;
        };
        self.push_endpoint_method_with_kind(
            Method::WorkspaceTodosList(WorkspaceTarget { workspace_id }),
            PendingEndpointKind::WorkspaceTodos,
            outcome,
        )
    }

    pub(super) fn add_todo(&mut self, label: String, outcome: &mut ClientShellInput) -> bool {
        let Some(workspace_id) = self.todo_workspace_id() else {
            return false;
        };
        self.push_endpoint_method_with_kind(
            Method::WorkspaceTodosAdd(WorkspaceTodoAddParams {
                workspace_id,
                label,
            }),
            PendingEndpointKind::WorkspaceTodos,
            outcome,
        )
    }

    pub(super) fn toggle_todo_done(
        &mut self,
        todo_id: u64,
        outcome: &mut ClientShellInput,
    ) -> bool {
        let Some(workspace_id) = self.todo_workspace_id() else {
            return false;
        };
        let Some(todo) = self.todo_data.items.iter().find(|todo| todo.id == todo_id) else {
            return false;
        };
        // Rapid presses queue absolute settings on the endpoint's FIFO command lane.
        // The checkbox itself always displays the last server-confirmed value.
        let last_requested = self
            .pending_requests
            .values()
            .filter_map(|request| match request.kind {
                PendingEndpointKind::SetTodoDone {
                    todo_id: target,
                    sequence,
                    done,
                } if target == todo_id => Some((sequence, done)),
                _ => None,
            })
            .max_by_key(|(sequence, _)| *sequence);
        let done = !last_requested.map_or(todo.done, |(_, done)| done);
        self.push_endpoint_method_with_kind(
            Method::WorkspaceTodosSetDone(WorkspaceTodoSetDoneParams {
                workspace_id,
                todo_id,
                done,
            }),
            PendingEndpointKind::SetTodoDone {
                todo_id,
                done,
                sequence: self.next_request_id,
            },
            outcome,
        )
    }

    pub(super) fn delete_todo(&mut self, todo_id: u64, outcome: &mut ClientShellInput) -> bool {
        let Some(workspace_id) = self.todo_workspace_id() else {
            return false;
        };
        self.push_endpoint_method_with_kind(
            Method::WorkspaceTodosDelete(WorkspaceTodoTarget {
                workspace_id,
                todo_id,
            }),
            PendingEndpointKind::WorkspaceTodos,
            outcome,
        )
    }

    pub(super) fn complete_todos(
        &mut self,
        result: Result<ResponseResult, ClientShellEndpointError>,
    ) -> bool {
        let (workspace_id, revision, todos) = match result {
            Ok(ResponseResult::WorkspaceTodos {
                workspace_id,
                revision,
                todos,
            }) => (workspace_id, revision, todos),
            Ok(_) => {
                self.set_endpoint_error("endpoint returned an unexpected workspace todo result");
                return true;
            }
            Err(_) => return true,
        };
        if self
            .snapshot
            .as_deref()
            .and_then(|s| s.focused_workspace_id.as_deref())
            != Some(workspace_id.as_str())
            || self.todo_data.revision.is_some_and(|last| revision <= last)
        {
            return false;
        }
        // Stable IDs increase in creation order. If the selected task disappeared, its
        // successor takes its place, including when another client deleted several rows.
        let previous = self.todo_data.selected;
        let selected = todos
            .iter()
            .find(|todo| previous.is_none_or(|id| todo.id >= id))
            .or_else(|| todos.last())
            .map(|todo| todo.id);
        self.todo_data.items = todos;
        self.todo_data.revision = Some(revision);
        self.todo_data.selected = selected;
        if previous != selected {
            self.reveal_selected_todo();
        }
        true
    }
}
