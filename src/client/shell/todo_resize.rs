use super::*;
use crate::api::schema::PaneDirection;

#[derive(Clone, Copy)]
pub(super) enum TodoDrag {
    SidebarHeight,
    Overlay {
        initial: Rect,
        pointer: (u16, u16),
        horizontal: i32,
        vertical: i32,
    },
}

// Ten rows keep the Spaces and Agents sections usable above their shared divider.
pub(super) fn docked_height(requested: u16, available: u16) -> u16 {
    if available < 14 {
        0
    } else {
        requested.clamp(4, available - 10)
    }
}

impl preferences::TodoSizePreferences {
    fn overlay_area(self, screen: Rect) -> Rect {
        let default = crate::ui::centered_popup_rect(screen, 54, 12).unwrap_or(screen);
        let maximum = crate::ui::centered_popup_rect(screen, u16::MAX, u16::MAX).unwrap_or(screen);
        let width = self
            .todo_overlay_width
            .unwrap_or(default.width)
            .clamp(12.min(maximum.width), maximum.width);
        let height = self
            .todo_overlay_height
            .unwrap_or(default.height)
            .clamp(5.min(maximum.height), maximum.height);
        Rect::new(
            screen.x + (screen.width - width) / 2,
            screen.y + (screen.height - height) / 2,
            width,
            height,
        )
    }
}

impl ClientShellState {
    pub(super) fn todo_overlay_area(&self, screen: Rect) -> Rect {
        self.todo_size.overlay_area(screen)
    }

    fn set_todo_overlay_size(&mut self, screen: Rect, width: u16, height: u16) -> bool {
        let current = self.todo_overlay_area(screen);
        let next = preferences::TodoSizePreferences {
            todo_overlay_width: Some(width),
            todo_overlay_height: Some(height),
            ..self.todo_size
        }
        .overlay_area(screen);
        // Resizing one axis preserves a saved size temporarily clipped on the other.
        if next.width != current.width {
            self.todo_size.todo_overlay_width = Some(next.width);
        }
        if next.height != current.height {
            self.todo_size.todo_overlay_height = Some(next.height);
        }
        next != current
    }

    fn set_todo_sidebar_height(&mut self, current: Rect, requested: u16, available: u16) -> bool {
        let height = docked_height(requested, available);
        if height == 0 || height == current.height {
            return false;
        }
        self.todo_size.todo_height = Some(height);
        true
    }

    pub(super) fn resize_todos(
        &mut self,
        direction: PaneDirection,
        outcome: &mut ClientShellInput,
    ) {
        let Some((cols, rows)) = self.last_composed_size else {
            return;
        };
        let screen = Rect::new(0, 0, cols, rows);
        let sidebar = self.layout(cols, rows).sidebar;
        let todos = self.todo_sidebar_area(sidebar);
        let step = |cells: u16| ((f32::from(cells) * 0.05).round() as u16).max(1);
        let horizontal = step(cols);
        let vertical = step(if todos.is_empty() {
            rows
        } else {
            sidebar.height
        });
        let changed = if todos.is_empty() {
            let current = self.todo_overlay_area(screen);
            let (width, height) = match direction {
                PaneDirection::Left => (current.width.saturating_sub(horizontal), current.height),
                PaneDirection::Right => (current.width.saturating_add(horizontal), current.height),
                PaneDirection::Up => (current.width, current.height.saturating_add(vertical)),
                PaneDirection::Down => (current.width, current.height.saturating_sub(vertical)),
            };
            self.set_todo_overlay_size(screen, width, height)
        } else {
            match direction {
                PaneDirection::Left => {
                    self.set_sidebar_width(sidebar.width.saturating_sub(horizontal), outcome)
                }
                PaneDirection::Right if sidebar.width < cols.saturating_sub(1) => self
                    .set_sidebar_width(
                        sidebar
                            .width
                            .saturating_add(horizontal)
                            .min(cols.saturating_sub(1)),
                        outcome,
                    ),
                PaneDirection::Right => false,
                PaneDirection::Up => self.set_todo_sidebar_height(
                    todos,
                    todos.height.saturating_add(vertical),
                    sidebar.height,
                ),
                PaneDirection::Down => self.set_todo_sidebar_height(
                    todos,
                    todos.height.saturating_sub(vertical),
                    sidebar.height,
                ),
            }
        };
        if changed {
            self.reveal_resized_todo(cols, rows);
            outcome.repaint = true;
            self.persist_chrome_preferences(outcome);
        }
    }

    pub(super) fn begin_todo_drag(&mut self, point: (u16, u16)) -> bool {
        let area = self.hits.todos.area;
        if !contains(area, point) {
            return false;
        }
        let drag = if self.hits.todos.sidebar_height > 0 {
            if point.1 != area.y {
                return false;
            }
            TodoDrag::SidebarHeight
        } else {
            if !self.todos_focused() || !todos::overlay_has_border(area) {
                return false;
            }
            let horizontal = i32::from(point.0 + 1 == area.right()) - i32::from(point.0 == area.x);
            let vertical = i32::from(point.1 + 1 == area.bottom()) - i32::from(point.1 == area.y);
            if horizontal == 0 && vertical == 0 {
                return false;
            }
            TodoDrag::Overlay {
                initial: area,
                pointer: point,
                horizontal,
                vertical,
            }
        };
        self.chrome_drag = Some(ClientChromeDrag::Todos(drag));
        true
    }

    pub(super) fn drag_todos(
        &mut self,
        drag: TodoDrag,
        point: (u16, u16),
        outcome: &mut ClientShellInput,
    ) {
        let Some((cols, rows)) = self.last_composed_size else {
            return;
        };
        let screen = Rect::new(0, 0, cols, rows);
        let sidebar = self.layout(cols, rows).sidebar;
        let todos = self.todo_sidebar_area(sidebar);
        let changed = match drag {
            TodoDrag::SidebarHeight if !todos.is_empty() => self.set_todo_sidebar_height(
                todos,
                sidebar.bottom().saturating_sub(point.1),
                sidebar.height,
            ),
            TodoDrag::Overlay {
                initial,
                pointer,
                horizontal,
                vertical,
            } if todos.is_empty() && self.todos_focused() => {
                // Both edges of a centered overlay move when its size changes.
                let dimension = |size: u16, at: u16, from: u16, edge: i32| {
                    let delta = i32::from(at) - i32::from(from);
                    (i32::from(size) + delta * edge * 2i32).clamp(0, i32::from(u16::MAX)) as u16
                };
                // Each untouched axis keeps its current effective size, even if the terminal resized mid-drag.
                let current = self.todo_overlay_area(screen);
                let width = if horizontal == 0 {
                    current.width
                } else {
                    dimension(initial.width, point.0, pointer.0, horizontal)
                };
                let height = if vertical == 0 {
                    current.height
                } else {
                    dimension(initial.height, point.1, pointer.1, vertical)
                };
                self.set_todo_overlay_size(screen, width, height)
            }
            _ => {
                self.chrome_drag = None;
                false
            }
        };
        if changed {
            if self.todos_focused() {
                self.reveal_resized_todo(cols, rows);
            }
            outcome.repaint = true;
        }
    }
}
