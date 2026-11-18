use super::*;
use crate::client::shell::{
    tests::{snapshot, surface},
    ClientShellAction, ClientShellConfig,
};
use crate::config::Config;
use crate::protocol::ClientShellAgent;

fn fixture() -> ClientShellSnapshot {
    let mut snapshot = snapshot();
    let mut workspace = snapshot.workspaces[0].clone();
    workspace.workspace_id = "ws_2".into();
    workspace.active_tab_id = "tab_3".into();
    workspace.number = 2;
    workspace.focused = false;
    snapshot.workspaces.push(workspace);
    for (id, ws) in [("tab_2", "ws_1"), ("tab_3", "ws_2")] {
        let mut tab = snapshot.tabs[0].clone();
        tab.tab_id = id.into();
        tab.workspace_id = ws.into();
        tab.focused = false;
        snapshot.tabs.push(tab);
    }
    for (id, ws, tab, status) in [
        ("pane_1", "ws_1", "tab_1", AgentStatus::Idle),
        ("pane_2", "ws_1", "tab_1", AgentStatus::Done),
        ("pane_3", "ws_1", "tab_2", AgentStatus::Blocked),
        ("pane_4", "ws_2", "tab_3", AgentStatus::Blocked),
        ("pane_5", "ws_2", "tab_3", AgentStatus::Working),
        ("pane_6", "ws_2", "tab_3", AgentStatus::Unknown),
        ("pane_7", "ws_2", "tab_3", AgentStatus::Idle),
    ] {
        if id != "pane_1" {
            let mut pane = snapshot.panes[0].clone();
            pane.pane_id = id.into();
            pane.workspace_id = ws.into();
            pane.tab_id = tab.into();
            pane.focused = false;
            snapshot.panes.push(pane);
        }
        snapshot.agents.push(ClientShellAgent {
            pane_id: id.into(),
            workspace_id: ws.into(),
            tab_id: tab.into(),
            name: None,
            display_agent: None,
            agent: None,
            title: None,
            terminal_title: None,
            terminal_title_stripped: None,
            agent_status: status,
            state_change_seq: 2,
            state_labels: vec![],
            tokens: vec![],
            focused: id == "pane_1",
        });
    }
    snapshot
}

fn choose(cycle: &mut AttentionCycle, snapshot: &mut ClientShellSnapshot) -> String {
    let target = cycle.next(snapshot).expect("eligible target");
    cycle.observe_focus(snapshot.focused_pane_id.as_deref(), Some(&target));
    snapshot.focused_pane_id = Some(target.clone());
    target
}

#[test]
fn attention_filters_and_orders_by_status_workspace_tab_and_pane() {
    let mut snapshot = fixture();
    snapshot.agents.reverse(); // Agent sidebar order is not attention order.
    snapshot.agent_view_label = Some("filtered".into());
    snapshot.agent_order = vec!["pane_5".into()];
    assert_eq!(
        candidates(&snapshot)
            .iter()
            .map(|t| t.pane_id.as_str())
            .collect::<Vec<_>>(),
        ["pane_3", "pane_4", "pane_2"]
    );
    snapshot.workspaces.reverse();
    assert_eq!(candidates(&snapshot)[0].pane_id, "pane_4");
    // A stale agent row, or a pane with a missing parent, cannot be focused.
    snapshot.panes.retain(|p| p.pane_id != "pane_4");
    snapshot.tabs.retain(|t| t.tab_id != "tab_2");
    assert_eq!(
        candidates(&snapshot)
            .iter()
            .map(|t| t.pane_id.as_str())
            .collect::<Vec<_>>(),
        ["pane_2"]
    );
}

#[test]
fn attention_visits_each_target_once_before_wrap_and_skips_focus() {
    let mut snapshot = fixture();
    let mut cycle = AttentionCycle::default();
    for expected in ["pane_3", "pane_4", "pane_2", "pane_3", "pane_4"] {
        assert_eq!(choose(&mut cycle, &mut snapshot), expected);
    }
    let mut cycle = AttentionCycle::default();
    snapshot.focused_pane_id = Some("pane_3".into());
    assert_eq!(choose(&mut cycle, &mut snapshot), "pane_4");
    snapshot.agents.retain(|a| a.pane_id == "pane_4");
    assert_eq!(cycle.next(&snapshot), None);
    snapshot.agents.clear();
    assert_eq!(cycle.next(&snapshot), None);
}

#[test]
fn attention_appends_new_states_and_prunes_resolved_and_closed_targets() {
    let mut snapshot = fixture();
    let mut cycle = AttentionCycle::default();
    assert_eq!(choose(&mut cycle, &mut snapshot), "pane_3");
    snapshot.agents[4].agent_status = AgentStatus::Blocked;
    snapshot.agents[4].state_change_seq += 1;
    assert_eq!(choose(&mut cycle, &mut snapshot), "pane_4");
    // A newly blocked target is appended behind the current cycle's done target.
    assert_eq!(choose(&mut cycle, &mut snapshot), "pane_2");
    snapshot.agents[2].state_change_seq += 1; // Same pane, new blocker.
    assert_eq!(choose(&mut cycle, &mut snapshot), "pane_5");
    assert_eq!(choose(&mut cycle, &mut snapshot), "pane_3");
    snapshot.agents[1].agent_status = AgentStatus::Idle;
    snapshot.panes.retain(|p| p.pane_id != "pane_4");
    assert_eq!(choose(&mut cycle, &mut snapshot), "pane_5");
}

#[test]
fn attention_handles_rapid_input_and_external_manual_focus() {
    let snapshot = fixture();
    let mut cycle = AttentionCycle::default();
    assert_eq!(cycle.next(&snapshot).as_deref(), Some("pane_3"));
    assert_eq!(cycle.next(&snapshot).as_deref(), Some("pane_4"));
    cycle.observe_focus(Some("pane_1"), Some("pane_4"));
    assert!(cycle.pending_focus.is_empty());
    assert_eq!(cycle.next(&snapshot).as_deref(), Some("pane_2"));
    cycle.observe_focus(Some("pane_4"), Some("pane_7"));
    assert!(cycle.visited.is_empty());
    assert_eq!(cycle.next(&snapshot).as_deref(), Some("pane_3"));
}

fn shell() -> ClientShellState {
    let config: Config = toml::from_str(
        r#"
        onboarding = false
        [keys]
        prefix = ["ctrl+a", "ctrl+b"]
        next_attention_agent = ["ctrl+alt+a", "prefix+a"]
        next_tab = ["prefix+n", "ctrl+shift+right", "alt+pageup"]
        previous_tab = ["prefix+p", "ctrl+shift+left", "alt+pagedown"]
    "#,
    )
    .unwrap();
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    let mut baseline = fixture();
    baseline.agents[1].agent_status = AgentStatus::Working;
    baseline.agents[1].state_change_seq = 1;
    state.set_snapshot(Box::new(baseline));
    let mut completed = fixture();
    completed.revision = 2;
    state.set_snapshot(Box::new(completed));
    state
}

fn key(state: &mut ClientShellState, bytes: &[u8]) -> ClientShellInput {
    state.handle_raw_events(crate::raw_input::parse_raw_input_bytes_sync(bytes))
}

fn focused_target(outcome: &ClientShellInput) -> &str {
    match outcome.actions.as_slice() {
        [ClientShellAction::Endpoint { request, .. }] => match &request.method {
            Method::PaneFocus(target) => &target.pane_id,
            Method::TabFocus(target) => &target.tab_id,
            other => panic!("unexpected method {other:?}"),
        },
        other => panic!("unexpected actions {other:?}"),
    }
}

#[test]
fn attention_direct_and_both_prefixes_share_cycle_and_emit_existing_pane_focus() {
    let mut state = shell();
    assert_eq!(focused_target(&key(&mut state, b"\x1b[97;7u")), "pane_3");
    assert_eq!(focused_target(&key(&mut state, b"\x01a")), "pane_4");
    assert_eq!(focused_target(&key(&mut state, b"\x02a")), "pane_2");
    // Manual navigation resets immediately, even before the server responds.
    key(&mut state, b"\x02n");
    assert_eq!(focused_target(&key(&mut state, b"\x1b[97;7u")), "pane_3");
}

#[test]
fn attention_resets_on_reconnect_generation_and_session_boot() {
    let mut state = shell();
    key(&mut state, b"\x01a");
    state.mark_endpoint_disconnected(&super::super::ClientEndpointId::Local);
    assert!(state.attention.visited.is_empty());
    state.set_endpoint_status(
        &super::super::ClientEndpointId::Local,
        super::super::ClientEndpointStatus::Online,
    );
    key(&mut state, b"\x01a");
    state.apply_active_snapshot(Box::new(fixture()), Some(2));
    assert!(state.attention.visited.is_empty());
    key(&mut state, b"\x01a");
    let mut next_session = fixture();
    next_session.boot_id = "other-session".into();
    state.apply_active_snapshot(Box::new(next_session), Some(2));
    assert!(state.attention.visited.is_empty());
}

#[test]
fn attention_completion_visibility_remains_independent_in_two_clients() {
    let mut viewing = shell();
    let background = shell();
    let mut shown = surface();
    shown.projection_revision = 2;
    shown.panes[0].pane_id = "pane_2".into();
    viewing.set_pane_surface(shown);
    assert!(!candidates(viewing.snapshot.as_deref().unwrap())
        .iter()
        .any(|t| t.pane_id == "pane_2"));
    assert!(candidates(background.snapshot.as_deref().unwrap())
        .iter()
        .any(|t| t.pane_id == "pane_2"));
}

#[test]
fn attention_modified_page_keys_and_existing_tab_shortcuts_wrap_without_reordering() {
    for (bytes, initial, expected) in [
        (b"\x1b[5;3~".as_slice(), "tab_1", "tab_2"),
        (b"\x1b[5;3:1~".as_slice(), "tab_2", "tab_1"),
        (b"\x1b[6;3~".as_slice(), "tab_1", "tab_2"),
        (b"\x1b[6;3:1~".as_slice(), "tab_2", "tab_1"),
        (b"\x02n".as_slice(), "tab_2", "tab_1"),
        (b"\x01p".as_slice(), "tab_1", "tab_2"),
        (b"\x1b[1;6C".as_slice(), "tab_2", "tab_1"),
        (b"\x1b[1;6D".as_slice(), "tab_1", "tab_2"),
    ] {
        let mut state = shell();
        state.snapshot.as_mut().unwrap().focused_tab_id = Some(initial.into());
        assert_eq!(
            focused_target(&key(&mut state, bytes)),
            expected,
            "{bytes:?}"
        );
        assert_eq!(state.snapshot.as_ref().unwrap().tabs[0].tab_id, "tab_1");
    }
}
