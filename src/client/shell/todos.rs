use super::*;
use crossterm::event::{KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::widgets::{Block, Borders, Clear, Widget};

#[derive(Default)]
pub(super) struct TodoUi {
    pub(super) editor: TextEditor,
    scroll: usize,
    return_pane: Option<String>,
    // Cover the interval between leaving Todos and the next focus projection.
    pending_focus: Option<(String, String)>,
}

#[derive(Default)]
pub(super) struct TodoHits {
    pub(super) area: Rect,
    body: Rect,
    add: Rect,
    // Only visible rows have hit targets. Task IDs survive concurrent list edits.
    rows: Vec<(Rect, u64)>,
    pub(super) sidebar_height: u16,
    pub(super) cursor: Option<crate::protocol::CursorState>,
}

impl ClientShellState {
    pub(super) fn todos_focused(&self) -> bool {
        matches!(
            self.mode,
            ClientShellMode::Todos
                | ClientShellMode::TodoPrefix
                | ClientShellMode::TodoResize
                | ClientShellMode::TodoEditor
        )
    }

    pub(super) fn todo_sidebar_area(&self, sidebar: Rect) -> Rect {
        // Leave room for the workspace controls, draggable divider and agent rows.
        if self.sidebar_collapsed || sidebar.width < 12 || !self.todos_available() {
            return Rect::default();
        }
        let requested = self
            .todo_size
            .todo_height
            .unwrap_or((sidebar.height / 4).clamp(4, 10));
        let height = todo_resize::docked_height(requested, sidebar.height);
        if height == 0 {
            return Rect::default();
        }
        // Keep the bottom-right collapse handle and vertical divider clickable.
        Rect::new(
            sidebar.x,
            sidebar.bottom() - height,
            sidebar.width - 2,
            height,
        )
    }

    pub(super) fn reset_todo_ui(&mut self) {
        if self.todos_focused() {
            self.mode = ClientShellMode::Terminal;
        }
        if matches!(self.chrome_drag, Some(ClientChromeDrag::Todos(_))) {
            self.chrome_drag = None;
        }
        self.todo_ui = TodoUi::default();
        self.hits.todos = TodoHits::default();
    }

    pub(super) fn focus_todos(&mut self, outcome: &mut ClientShellInput) -> bool {
        if self.todos_focused() {
            return true;
        }
        if !self.refresh_todos(outcome) {
            return false;
        }
        if self.copy_mode.is_some() {
            self.exit_copy_mode(false, outcome);
        }
        self.todo_ui.return_pane = self.input_pane_id();
        self.navigate_workspace_id = None;
        self.mode = ClientShellMode::Todos;
        self.reveal_selected_todo();
        outcome.repaint = true;
        true
    }

    pub(super) fn remember_todo_exit_focus(&mut self, actions: &[ClientShellAction]) {
        for action in actions {
            if let ClientShellAction::Endpoint { request, .. } = action {
                if let crate::api::schema::Method::PaneFocus(target) = &request.method {
                    if self
                        .snapshot
                        .as_ref()
                        .and_then(|s| s.focused_pane_id.as_ref())
                        != Some(&target.pane_id)
                    {
                        self.todo_ui.pending_focus =
                            Some((request.id.clone(), target.pane_id.clone()));
                    }
                }
            }
        }
    }

    pub(super) fn input_pane_id(&self) -> Option<String> {
        self.todo_ui
            .pending_focus
            .as_ref()
            .map(|(_, pane)| pane.clone())
            .or_else(|| self.focused_pane_id())
    }

    pub(super) fn cancel_pending_todo_focus(&mut self, request: Option<&str>) {
        if self
            .todo_ui
            .pending_focus
            .as_ref()
            .is_some_and(|(id, _)| request.is_none_or(|r| r == id))
        {
            self.todo_ui.pending_focus = None;
        }
    }

    pub(super) fn reconcile_todo_focus(&mut self, snapshot: &ClientShellSnapshot) {
        if let Some((_, target)) = &self.todo_ui.pending_focus {
            if self
                .snapshot
                .as_ref()
                .and_then(|s| s.focused_pane_id.as_ref())
                != snapshot.focused_pane_id.as_ref()
                || !snapshot.panes.iter().any(|pane| pane.pane_id == *target)
            {
                self.todo_ui.pending_focus = None;
            }
        }
    }

    fn return_from_todos(&mut self, outcome: &mut ClientShellInput) {
        self.mode = ClientShellMode::Terminal;
        if let Some(pane_id) = self.todo_ui.return_pane.take().filter(|id| {
            self.snapshot.as_deref().is_some_and(|snapshot| {
                snapshot.focused_pane_id.as_ref() != Some(id)
                    && snapshot.panes.iter().any(|pane| pane.pane_id == *id)
            })
        }) {
            self.push_endpoint_method(
                crate::api::schema::Method::PaneFocus(crate::api::schema::PaneTarget { pane_id }),
                outcome,
            );
        }
        outcome.repaint = true;
    }

    pub(super) fn reveal_resized_todo(&mut self, cols: u16, rows: u16) {
        let sidebar = self.todo_sidebar_area(self.layout(cols, rows).sidebar);
        let content_height = if sidebar.is_empty() {
            let area = self.todo_overlay_area(Rect::new(0, 0, cols, rows));
            area.height
                .saturating_sub(2 * u16::from(overlay_has_border(area)))
        } else {
            sidebar.height - 1
        };
        self.reveal_selected_todo_in(
            content_height.saturating_sub(1 + u16::from(self.mode == ClientShellMode::TodoEditor)),
        );
    }

    pub(super) fn reveal_selected_todo(&mut self) {
        self.reveal_selected_todo_in(self.hits.todos.body.height);
    }

    fn reveal_selected_todo_in(&mut self, rows: u16) {
        let Some(index) = self
            .todo_data
            .items
            .iter()
            .position(|item| Some(item.id) == self.todo_data.selected)
        else {
            self.todo_ui.scroll = 0;
            return;
        };
        self.reveal_todo_at(index, rows);
    }

    fn reveal_todo_at(&mut self, index: usize, rows: u16) {
        let height = usize::from(rows.max(1));
        self.todo_ui.scroll = self
            .todo_ui
            .scroll
            .clamp(index.saturating_sub(height - 1), index);
    }

    fn move_todo_selection(&mut self, down: bool) {
        if self.todo_data.items.is_empty() {
            return;
        }
        let index = self
            .todo_data
            .items
            .iter()
            .position(|item| Some(item.id) == self.todo_data.selected);
        let next = match index {
            None => 0,
            Some(i) if down => i.saturating_add(1).min(self.todo_data.items.len() - 1),
            Some(i) => i.saturating_sub(1),
        };
        self.todo_data.selected = Some(self.todo_data.items[next].id);
        self.reveal_todo_at(next, self.hits.todos.body.height);
    }

    fn start_todo_editor(&mut self) {
        self.todo_ui.editor = TextEditor::default();
        self.mode = ClientShellMode::TodoEditor;
    }

    pub(super) fn route_todo_key(
        &mut self,
        key: &crate::input::TerminalKey,
        outcome: &mut ClientShellInput,
    ) {
        if self.mode == ClientShellMode::TodoEditor {
            match key.code {
                KeyCode::Esc if key.kind == KeyEventKind::Press => {
                    self.mode = ClientShellMode::Todos
                }
                KeyCode::Enter if key.kind == KeyEventKind::Press => {
                    let label = self.todo_ui.editor.as_str().trim();
                    if !label.is_empty() && self.add_todo(label.to_owned(), outcome) {
                        self.mode = ClientShellMode::Todos;
                    }
                }
                _ => {
                    outcome.repaint |= self.todo_ui.editor.handle_key(key).is_some();
                    return;
                }
            }
        } else {
            if key.modifiers != KeyModifiers::NONE {
                return;
            }
            match key.code {
                KeyCode::Up | KeyCode::Down => self.move_todo_selection(key.code == KeyCode::Down),
                _ if key.kind != KeyEventKind::Press => return,
                KeyCode::Esc => self.return_from_todos(outcome),
                KeyCode::Char('n') => self.start_todo_editor(),
                KeyCode::Char(' ') => {
                    if let Some(id) = self.todo_data.selected {
                        self.toggle_todo_done(id, outcome);
                    }
                }
                KeyCode::Delete => {
                    if let Some(id) = self.todo_data.selected {
                        self.delete_todo(id, outcome);
                    }
                }
                _ => return,
            }
        }
        outcome.repaint = true;
    }

    pub(super) fn handle_todo_mouse(
        &mut self,
        mouse: MouseEvent,
        outcome: &mut ClientShellInput,
    ) -> bool {
        let point = (mouse.column, mouse.row);
        if self.config.mouse_capture && mouse.kind == MouseEventKind::Down(MouseButton::Left) {
            if self.begin_todo_drag(point) {
                return true;
            }
            if self.hits.todos.sidebar_height > 0 && contains(self.hits.sidebar_divider, point) {
                // The checklist shares its right edge with the sidebar's width control.
                return false;
            }
        }
        if !contains(self.hits.todos.area, point)
            || !self.config.mouse_capture
            || (self.hits.todos.sidebar_height == 0 && !self.todos_focused())
        {
            if self.todos_focused() && mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                // Let the clicked pane, workspace or control take its normal action.
                self.todo_ui.return_pane = None;
                self.mode = ClientShellMode::Terminal;
                outcome.repaint = true;
            }
            return false;
        }
        match mouse.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let max_scroll = self
                    .todo_data
                    .items
                    .len()
                    .saturating_sub(usize::from(self.hits.todos.body.height));
                let first = self.todo_ui.scroll.min(max_scroll);
                self.todo_ui.scroll = match mouse.kind {
                    MouseEventKind::ScrollUp => {
                        first.saturating_sub(self.config.mouse_scroll_lines)
                    }
                    _ => first
                        .saturating_add(self.config.mouse_scroll_lines)
                        .min(max_scroll),
                };
                outcome.repaint |= self.todo_ui.scroll != first;
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let scroll = self.todo_ui.scroll;
                if !self.focus_todos(outcome) {
                    return true;
                }
                if contains(self.hits.todos.add, point) {
                    self.start_todo_editor();
                } else if let Some((row, id)) = self
                    .hits
                    .todos
                    .rows
                    .iter()
                    .copied()
                    .find(|(rect, _)| contains(*rect, point))
                {
                    if self.todo_data.items.iter().any(|item| item.id == id) {
                        self.mode = ClientShellMode::Todos;
                        self.todo_data.selected = Some(id);
                        self.todo_ui.scroll = scroll;
                        if point.0 < row.x.saturating_add(4) {
                            self.toggle_todo_done(id, outcome);
                        }
                    }
                }
                outcome.repaint = true;
            }
            _ => {}
        }
        true
    }
}

pub(super) fn render(
    buffer: &mut Buffer,
    area: Rect,
    shell: &ClientShellState,
    overlay: bool,
) -> TodoHits {
    if area.is_empty() {
        return TodoHits::default();
    }
    let palette = &shell.config.palette;
    let mut hits = TodoHits {
        area,
        sidebar_height: if overlay { 0 } else { area.height },
        ..TodoHits::default()
    };
    let inner = if overlay {
        Clear.render(area, buffer);
        let mut block =
            Block::default().style(Style::default().fg(palette.text).bg(palette.sidebar_bg));
        if overlay_has_border(area) {
            block = block
                .borders(Borders::ALL)
                .border_style(Style::default().fg(palette.accent));
        }
        let inner = block.inner(area);
        block.render(area, buffer);
        inner
    } else {
        for x in area.x..area.right() {
            buffer[(x, area.y)]
                .set_symbol("─")
                .set_fg(palette.surface_dim);
        }
        Rect::new(area.x, area.y + 1, area.width, area.height - 1)
    };
    if inner.is_empty() {
        return hits;
    }
    let title_color = if shell.todos_focused() {
        palette.accent
    } else {
        palette.overlay0
    };
    let title_style = Style::default()
        .fg(title_color)
        .add_modifier(Modifier::BOLD);
    render::put_text(buffer, inner.x, inner.y, inner.width, " todos", title_style);
    if shell.config.mouse_capture {
        hits.add = Rect::new(
            inner.right() - inner.width.min(3),
            inner.y,
            inner.width.min(3),
            1,
        );
        render::put_text(
            buffer,
            hits.add.x,
            hits.add.y,
            hits.add.width,
            " +",
            title_style,
        );
    }

    let editing = shell.mode == ClientShellMode::TodoEditor;
    let body_height = inner.height.saturating_sub(1 + u16::from(editing));
    let body = Rect::new(inner.x, inner.y.saturating_add(1), inner.width, body_height);
    let items = &shell.todo_data.items;
    let start = shell
        .todo_ui
        .scroll
        .min(items.len().saturating_sub(usize::from(body.height)));
    hits.body = body;
    for (offset, item) in items
        .iter()
        .skip(start)
        .take(usize::from(body.height))
        .enumerate()
    {
        let y = body.y + offset as u16;
        let row = Rect::new(body.x, y, body.width, 1);
        let selected = shell.todos_focused() && shell.todo_data.selected == Some(item.id);
        let style = Style::default().fg(if item.done {
            palette.overlay0
        } else {
            palette.text
        });
        render::put_text(
            buffer,
            row.x,
            y,
            row.width.min(4),
            if item.done { " [x]" } else { " [ ]" },
            style,
        );
        render::put_text(
            buffer,
            row.x.saturating_add(5),
            y,
            row.width.saturating_sub(5),
            &item.label,
            if selected {
                style.add_modifier(Modifier::BOLD)
            } else {
                style
            },
        );
        if selected {
            buffer.set_style(row, Style::default().bg(palette.active_row_bg));
        }
        hits.rows.push((row, item.id));
    }
    if editing {
        let y = inner.bottom() - 1;
        let left = u16::from(inner.width > 2) * 2;
        render::put_text(buffer, inner.x, y, left, "> ", title_style);
        hits.cursor = text_editor::render(
            buffer,
            Rect::new(inner.x + left, y, inner.width - left, 1),
            &shell.todo_ui.editor,
            Style::default().fg(palette.text).bg(palette.panel_bg),
        );
    }
    hits
}

pub(super) fn overlay_has_border(area: Rect) -> bool {
    area.height >= 4 && area.width >= 8
}

#[cfg(test)]
#[path = "tests/todo_controls.rs"]
mod tests;
