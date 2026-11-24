use serde::{Deserialize, Serialize};

use crate::api::schema::WorkspaceTodo;

/// Stored with the workspace. Deleting the last task preserves both counters.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct WorkspaceTodoList {
    next_id: u64,
    pub(crate) revision: u64,
    pub(crate) items: Vec<WorkspaceTodo>,
}

impl Default for WorkspaceTodoList {
    fn default() -> Self {
        Self {
            next_id: 1,
            revision: 0,
            items: Vec::new(),
        }
    }
}

impl WorkspaceTodoList {
    pub(crate) fn is_pristine(&self) -> bool {
        self.next_id == 1 && self.revision == 0 && self.items.is_empty()
    }

    pub(crate) fn add(&mut self, label: &str) -> Result<(), &'static str> {
        let trimmed = label.trim();
        if trimmed.is_empty()
            || label
                .chars()
                .any(|ch| ch.is_control() || matches!(ch, '\u{2028}' | '\u{2029}'))
        {
            return Err("label must contain nonblank text on a single line");
        }
        let id = self.next_id;
        let next_id = id.checked_add(1).ok_or("todo IDs exhausted")?;
        self.advance_revision()?;
        self.items.push(WorkspaceTodo {
            id,
            label: trimmed.to_owned(),
            done: false,
        });
        self.next_id = next_id;
        Ok(())
    }

    pub(crate) fn set_done(&mut self, id: u64, done: bool) -> Result<(), &'static str> {
        let index = self
            .items
            .iter()
            .position(|task| task.id == id)
            .ok_or("task does not exist in this workspace")?;
        if self.items[index].done != done {
            self.advance_revision()?;
            self.items[index].done = done;
        }
        Ok(())
    }

    pub(crate) fn delete(&mut self, id: u64) -> Result<(), &'static str> {
        if let Some(index) = self.items.iter().position(|task| task.id == id) {
            self.advance_revision()?;
            self.items.remove(index);
        }
        Ok(())
    }

    fn advance_revision(&mut self) -> Result<(), &'static str> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or("todo revisions exhausted")?;
        Ok(())
    }
}
