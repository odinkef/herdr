use super::*;

impl HeadlessServer {
    /// Public numbers change at the commit boundary. Relocate session ownership before
    /// ordinary topology reconciliation can discard the retired tab ID.
    pub(super) fn relocate_transferred_tab_owners(&mut self, response: &str) {
        let Ok(api::schema::SuccessResponse {
            result:
                api::schema::ResponseResult::TabTransferred {
                    changed: true,
                    previous_workspace_id,
                    previous_tab_id,
                    tab,
                    ..
                },
            ..
        }) = serde_json::from_str(response)
        else {
            return;
        };
        if self.popup_owner_tab_id.as_ref() == Some(&previous_tab_id) {
            self.popup_owner_tab_id = Some(tab.tab_id.clone());
        }
        if let Some(controller) = self.tab_geometry_controllers.remove(&previous_tab_id) {
            self.tab_geometry_controllers
                .insert(tab.tab_id.clone(), controller);
        }
        let mut releases = Vec::new();
        for (&client_id, client) in &mut self.clients {
            let Some(location) = client.shell_location.as_mut() else {
                continue;
            };
            let viewing_transferred = location.focused_tab_id() == Some(&previous_tab_id);
            if location.active_tab_ids.get(&previous_workspace_id) == Some(&previous_tab_id) {
                location.active_tab_ids.remove(&previous_workspace_id);
            }
            if viewing_transferred {
                location.focus_tab(tab.workspace_id.clone(), tab.tab_id.clone());
                releases.push((client_id, client.drain_shell_held_inputs()));
                client.render_state.reset_baseline();
                client.request_recompute();
                client.shell_graphics_delivery = Default::default();
            }
        }
        for (client_id, held) in releases {
            self.release_client_shell_inputs(client_id, held);
        }
    }
}
