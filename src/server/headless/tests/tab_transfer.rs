use super::*;
use crate::api::schema::{
    Method, ResponseResult, SuccessResponse, TabTransferDestination, TabTransferParams,
};

fn transfer_server() -> HeadlessServer {
    let mut server = test_headless_server();
    let mut source = crate::workspace::Workspace::test_new("source");
    source.test_add_tab(Some("unaffected"));
    source.switch_tab(0);
    server.app.state.workspaces =
        vec![source, crate::workspace::Workspace::test_new("destination")];
    server.app.state.ensure_test_terminals();
    server.app.state.active = Some(0);
    server.app.state.selected = 0;
    server.app.state.mode = crate::app::Mode::Terminal;
    for terminal in server.app.state.terminals.keys() {
        let (runtime, _) = crate::terminal::TerminalRuntime::test_with_channel(80, 24);
        server
            .app
            .terminal_runtimes
            .insert(terminal.clone(), runtime);
    }
    server
}

fn transfer(
    server: &mut HeadlessServer,
    client_id: Option<u64>,
    params: TabTransferParams,
) -> ResponseResult {
    let (respond_to, response_rx) = std::sync::mpsc::channel();
    let msg = api::ApiRequestMessage {
        request: api::schema::Request {
            id: "transfer".into(),
            method: Method::TabTransfer(params),
        },
        respond_to,
        response_write_complete: None,
    };
    match client_id {
        Some(client_id) => {
            server.handle_client_shell_api_request(client_id, msg);
        }
        None => {
            server.handle_api_request_with_shutdown_check(msg);
        }
    }
    serde_json::from_str::<SuccessResponse>(&response_rx.recv().unwrap())
        .unwrap()
        .result
}

#[tokio::test]
async fn tab_transfer_viewers_follow_logical_tab_and_unaffected_viewers_keep_location() {
    let mut server = transfer_server();
    let source_tab = server.app.public_tab_id(0, 0).unwrap();
    let unaffected_tab = server.app.public_tab_id(0, 1).unwrap();
    let destination_workspace = server.app.public_workspace_id(1);
    let (control9, _render9) = connect_test_shell(&mut server, 9, 80, 23);
    let (control10, _render10) = connect_test_shell(&mut server, 10, 80, 23);
    let (control11, _render11) = connect_test_shell(&mut server, 11, 80, 23);
    client_shell_snapshot(&control9);
    client_shell_snapshot(&control10);
    client_shell_snapshot(&control11);
    assert!(server.focus_shell_client_on_tab(11, &unaffected_tab));
    server
        .tab_geometry_controllers
        .insert(source_tab.clone(), 9);
    let ResponseResult::TabTransferred {
        tab,
        changed: true,
        pane_id_map,
        ..
    } = transfer(
        &mut server,
        Some(9),
        TabTransferParams {
            tab_id: source_tab.clone(),
            destination: TabTransferDestination::Workspace {
                workspace_id: destination_workspace,
            },
            focus: false,
        },
    )
    else {
        panic!("expected transfer");
    };
    assert_eq!(
        server.shell_tab_id_for_client(9).as_deref(),
        Some(tab.tab_id.as_str())
    );
    assert_eq!(
        server.shell_tab_id_for_client(10).as_deref(),
        Some(tab.tab_id.as_str())
    );
    assert_eq!(
        server.shell_tab_id_for_client(11).as_deref(),
        Some(unaffected_tab.as_str())
    );
    assert_eq!(server.tab_geometry_controllers.get(&tab.tab_id), Some(&9));
    assert!(!server.tab_geometry_controllers.contains_key(&source_tab));

    server.render_and_stream();
    let mut relocation_seen = false;
    let mut snapshot_seen = false;
    for _ in 0..16 {
        let framed = control10
            .recv_timeout(Duration::from_secs(1))
            .expect("relocation and snapshot");
        if let ServerMessage::EndpointControl { kind, data } = read_server_message(framed) {
            if kind == protocol::endpoint::PANE_RELOCATIONS_KIND {
                assert!(!snapshot_seen, "aliases must precede affected snapshot");
                let relocation: protocol::endpoint::EndpointPaneRelocations =
                    serde_json::from_str(&data).unwrap();
                assert_eq!(relocation.aliases, pane_id_map);
                relocation_seen = true;
            } else if kind == protocol::endpoint::ENDPOINT_SNAPSHOT_KIND {
                assert!(
                    relocation_seen,
                    "snapshot cannot prune retired identities before aliases"
                );
                let snapshot: protocol::ClientShellSnapshot = serde_json::from_str(&data).unwrap();
                assert_eq!(
                    snapshot.focused_tab_id.as_deref(),
                    Some(tab.tab_id.as_str())
                );
                snapshot_seen = true;
                break;
            }
        }
    }
    assert!(relocation_seen && snapshot_seen);
    let revision = server.clients[&10].shell_pane_relocation_revision;
    server.render_and_stream();
    assert_eq!(server.clients[&10].shell_pane_relocation_revision, revision);
    for message in control10.try_iter().map(read_server_message) {
        assert!(
            !matches!(message, ServerMessage::EndpointControl {kind, ..} if kind == protocol::endpoint::PANE_RELOCATIONS_KIND)
        );
    }
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn public_tab_transfer_focus_changes_only_foreground_and_viewers_of_moved_tab() {
    let mut server = transfer_server();
    let source_tab = server.app.public_tab_id(0, 0).unwrap();
    let unaffected_tab = server.app.public_tab_id(0, 1).unwrap();
    let (control9, _render9) = connect_test_shell(&mut server, 9, 80, 23);
    let (control10, _render10) = connect_test_shell(&mut server, 10, 80, 23);
    client_shell_snapshot(&control9);
    client_shell_snapshot(&control10);
    server.focus_shell_client_on_tab(10, &unaffected_tab);
    server.foreground_client_id = Some(9);
    let ResponseResult::TabTransferred { tab, .. } = transfer(
        &mut server,
        None,
        TabTransferParams {
            tab_id: source_tab,
            destination: TabTransferDestination::NewWorkspace {
                label: Some("new".into()),
            },
            focus: true,
        },
    ) else {
        panic!("expected transfer");
    };
    assert_eq!(
        server.shell_tab_id_for_client(9).as_deref(),
        Some(tab.tab_id.as_str())
    );
    assert_eq!(
        server.shell_tab_id_for_client(10).as_deref(),
        Some(unaffected_tab.as_str())
    );
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
#[ignore = "manual two-client relocation metadata render/fanout profile; 1/15 populated panes"]
async fn pane_relocation_fanout_profile() {
    use ratatui::layout::Direction;
    const SAMPLES: usize = 45;
    const WARMUP: usize = 5;
    for pane_count in [1, 15] {
        for alias_count in [0, 4096] {
            let mut server = test_headless_server();
            let mut workspace = crate::workspace::Workspace::test_new("relocation-profile");
            let mut pane_ids = vec![workspace.tabs[0].root_pane];
            for index in 1..pane_count {
                workspace.tabs[0]
                    .layout
                    .focus_pane(pane_ids[(index - 1) / 2]);
                pane_ids.push(workspace.test_split(if index % 2 == 0 {
                    Direction::Vertical
                } else {
                    Direction::Horizontal
                }));
            }
            server.app.state.workspaces = vec![workspace];
            server.app.state.ensure_test_terminals();
            server.app.state.active = Some(0);
            server.app.state.selected = 0;
            server.app.state.mode = crate::app::Mode::Terminal;
            for terminal in server.app.state.terminals.keys() {
                server.app.terminal_runtimes.insert(
                    terminal.clone(),
                    crate::terminal::TerminalRuntime::test_with_screen_bytes(
                        120,
                        40,
                        b"populated terminal with scrollback\r\nready\r\n",
                    ),
                );
            }
            let (control1, render1) = connect_test_shell(&mut server, 1, 120, 40);
            let (control2, render2) = connect_test_shell(&mut server, 2, 120, 40);
            client_shell_snapshot(&control1);
            client_shell_snapshot(&control2);
            let current = server.app.public_pane_id(0, pane_ids[0]).unwrap();
            server.app.state.pane_relocation_aliases = (0..alias_count)
                .map(|index| (format!("retired-profile-pane-{index}"), current.clone()))
                .collect();
            server.app.state.pane_relocation_revision += 1;
            // Send the complete companion once, outside steady-state measurements.
            server.render_and_stream();
            client_shell_snapshot(&control1);
            client_shell_snapshot(&control2);
            render1.try_iter().for_each(drop);
            render2.try_iter().for_each(drop);
            let mut samples = Vec::new();
            for sample in 0..(SAMPLES + WARMUP) {
                let bytes = format!("\x1b[H{sample:03} populated terminal");
                for runtime in server.app.terminal_runtimes.values() {
                    runtime.test_process_pty_bytes(bytes.as_bytes());
                }
                let started = Instant::now();
                server.render_and_stream();
                let elapsed = started.elapsed();
                render1.try_iter().for_each(drop);
                render2.try_iter().for_each(drop);
                for framed in control1.try_iter().chain(control2.try_iter()) {
                    assert!(
                        !matches!(read_server_message(framed),
                            ServerMessage::EndpointControl {kind, ..}
                                if kind == protocol::endpoint::PANE_RELOCATIONS_KIND
                        ),
                        "unchanged mapping revision must not resend or rescan aliases"
                    );
                }
                assert_eq!(
                    server.clients[&1].shell_pane_relocation_revision,
                    Some(server.app.state.pane_relocation_revision)
                );
                assert_eq!(
                    server.clients[&2].shell_pane_relocation_revision,
                    Some(server.app.state.pane_relocation_revision)
                );
                if sample >= WARMUP {
                    samples.push(elapsed.as_micros());
                }
            }
            samples.sort_unstable();
            println!("pane_relocation_fanout panes={pane_count} clients=2 aliases={alias_count} median_us={} p95_us={}",
                samples[SAMPLES / 2], samples[SAMPLES * 95 / 100]);
            shutdown_test_runtimes(&mut server);
        }
    }
}
