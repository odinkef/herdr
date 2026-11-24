# Local attention-navigation fork

This branch is based on Herdr v0.9.5. Its client uses the existing `pane.focus`
API and completion acknowledgement, including when attached to an unmodified
v0.9.5 server. It adds no wire fields or protocol variants.

Merge these into `~/.config/herdr/config.toml`'s existing `[keys]` section:

```toml
[keys]
prefix = ["ctrl+a", "ctrl+b"]
next_attention_agent = ["ctrl+alt+a", "prefix+a"]
next_tab = ["prefix+n", "alt+pageup"]
previous_tab = ["prefix+p", "alt+pagedown"]
move_tab_previous = "ctrl+shift+left"
move_tab_next = "ctrl+shift+right"
move_tab_to_workspace = "prefix+shift+m"
```

Ctrl+Alt+A and either Ctrl+A then lowercase a or Ctrl+B then lowercase a share
one cycle over the current session. Each cycle starts with blocked agents,
then unviewed completions, in workspace/tab/pane order. Newly eligible targets
are appended to the current cycle; resolved or closed targets are pruned.
Each pane/state-change is visited once before wrapping. The focused pane is
skipped if there is another target; otherwise focus stays unchanged. Manual
navigation, switching session/endpoint, and reconnect reset the cycle.
Completion visibility follows the client display, independently in each client.
Agent sidebar sorting and filters do not restrict this session-wide shortcut.

Alt+PageUp selects the **next** tab; Alt+PageDown selects the **previous** tab.
Both wrap around without moving tabs. The existing prefix+n/p bindings also
switch tabs without reordering them. Ctrl+Shift+Left moves the current tab one
position left; Ctrl+Shift+Right moves it one position right. The moved tab keeps
focus, wraps from the first position to the last or vice versa, and stays put
when it is the only tab. Ghostty forwards both Ctrl+Shift+arrow shortcuts to
Herdr. PageUp and PageDown are case-insensitive configuration names, formatted
as `pageup` and `pagedown`.

To remove Ctrl+Alt+A, set `next_attention_agent = "prefix+a"`.
To remove prefix then a, set `next_attention_agent = "ctrl+alt+a"`.
To disable both, set `next_attention_agent = ""`.
After editing this configuration, reload with **prefix then r**. The binding is
unbound by default in other configurations.

## Move a live tab between spaces

Press Ctrl+A then Shift+M or Ctrl+B then Shift+M to open **Move to space…**.
The configurable action is `move_tab_to_workspace`; its default is
`prefix+shift+m`, so the existing prefix array enables both. Right-clicking an
inactive tab opens the same picker for that tab without selecting it. Type to
filter all other spaces on the same server, including collapsed groups, or
choose **New space…** to start its naming dialog with your search. Escape
cancels without changing focus. You can also drag a tab directly onto a visible
sidebar space, including a worktree child. The highlighted destination receives
the tab on release. Hovering does not switch spaces; other machines, headers,
blank areas, and the source space reject drops.

The move appends and follows the entire live tab, preserving its label, split
tree and ratios, zoom, focused pane, terminals, processes, directories,
scrollback, and agent state. The source space disappears only if empty. Other
worktree spaces and files stay in place, and a new destination starts no shell.
Ctrl+Shift+Left/Right keep their tab reorder behavior.

For scripts:

```bash
herdr tab move --current --workspace w2
herdr tab move w1:t2 --new-workspace --label review --no-focus
```

Read destination IDs from the response. Public tab and pane IDs change; live
process environments retain their original strings. Pane aliases preserve
supported `--current` addressing, and this command resolves the caller's current
tab rather than trusting its inherited tab/workspace environment variables.
The API is `tab.transfer`; existing `tab.move` remains reorder-only.

Both the client and its selected server must support this addition. An older
server remains usable and produces an unsupported-action notice. Picker and
drag captures expire on reconnect or server replacement; uncertain requests
are never retried automatically.

The local `fork-integrated` branch combines these controls with workspace
Todos. Press either configured prefix, then `t`, to open the active space's
checklist. While the list is focused, Alt+Shift+Up/Down changes its height and
Alt+Shift+Left/Right changes the sidebar width. Press either prefix, then
Shift+R, to enter resize mode. Drag the divider above Todos or the right edge
of the sidebar to resize it with the mouse.

Tabs moved into another space show that space's list. A newly created space
starts with an empty list. See the [fork installation guide](../../FORK_SETUP.md)
for building the combined branch, backing up session state, and activating it.

## Rebuild and validate

Install Zig 0.16.0 at `~/.local/opt/zig-0.16.0`, verifying the archive checksum
against <https://ziglang.org/download/index.json>. Rustup installs the pinned
Rust toolchain from `rust-toolchain.toml`. Repository checks also use `just`,
`cargo-nextest`, Bun, Python 3.11 or newer, and a Git version supporting
`git diff --default-prefix`. On this workstation the newer Python and test-only
Git are isolated from the system tools. Before running checks, use:

```bash
export PATH="/home/odin/herdr/.local/attention-navigation/git-tool/bin:/home/odin/.local/share/uv/python/cpython-3.12.16-linux-x86_64-gnu/bin:$PATH"
```

```bash
cd /home/odin/herdr
export ZIG="$HOME/.local/opt/zig-0.16.0/zig"
cargo fmt --check
just ci
just docs-contract-test
python3 scripts/config_reference_check.py
cargo build --release --locked
```

`just check` additionally runs Windows cross-compilation and requires a Windows
SDK/CRT installation; see `just setup-windows-cross`. Native Linux builds do
not need that SDK. Do not substitute another Zig version.

Test the release client in a disposable named session before replacing the
installed binary. Clear inherited `HERDR_SOCKET_PATH` and
`HERDR_CLIENT_SOCKET_PATH` when starting an isolated server. Keep existing
servers and user panes running.

## Install and roll back

Back up both `~/.local/bin/herdr` and `~/.config/herdr/config.toml`. Stage the
release binary beside the installed executable and rename it atomically:

```bash
install -m 755 target/release/herdr "$HOME/.local/bin/herdr.new"
mv -f "$HOME/.local/bin/herdr.new" "$HOME/.local/bin/herdr"
systemctl --user restart app-com.mitchellh.ghostty.dropdown.service
```

Restart only this dropdown client service. Existing Herdr servers keep their
original executable and continue hosting their panes; existing tmux sessions
are unaffected. Check F1 show/hide and placement plus all four shortcuts in
Ghostty. To roll back, stage the backed-up binary beside `herdr`, rename it over
`herdr`, restore the backed-up config, and restart the same client service.
An upstream `herdr update` can replace this local build; rebuild this branch to
restore its additions.


## Validation on this workstation

The Linux release build passed formatting, Clippy (`--all-targets -D warnings`),
4,039 Rust tests (14 existing skips), 150 Python maintenance tests, the release
workflow tests, UI hot-path architecture tests, integration-asset tests, config
reference validation, and documentation contract tests. The Windows lint stage
of `just check` was not run successfully because no Windows SDK is installed;
native Linux validation was completed separately.

A disposable PTY session and a separate Ghostty window both exercised the
release client against the original, unmodified v0.9.5 server. They checked both
attention bindings and both prefix keys, completion acknowledgement, no-target
behavior, Alt+PageUp/PageDown, tab wraparound, and existing tab shortcuts. Logs
and test harnesses are in `.local/attention-navigation/` (ignored local files).


The installed binary and personal configuration were backed up to
`~/.local/share/herdr/backups/attention-navigation-20261113T141544Z/`.
Only `app-com.mitchellh.ghostty.dropdown.service` was restarted. The existing
server PID 297788, all 16 pane/terminal identities, 7 tabs, 2 workspaces, and
focused pane were preserved. F1 show/hide was verified after installation at
the original geometry (2560×1413 at 5120,54), retaining above/sticky and
skip-taskbar/skip-pager window properties. No local tmux server/client processes
were present in either process inventory.

### Tab-reorder configuration, 18 November 2026

Only the four tab shortcut settings were changed. Both prefixes, attention
shortcuts, and all other settings were preserved. The previous configuration
is backed up at
`~/.local/share/herdr/backups/tab-reorder-20261118T130657377372Z/config.toml`.
The candidate and installed configurations both passed `herdr config check`
without diagnostics or binding conflicts; documentation contract tests passed.

A disposable Ghostty session passed 20 checks covering tab movement, focus
preservation, wrapping, a single tab, both prefixes, tab switching, reload, and
help entries. An isolated PTY using Ghostty's enhanced key sequences passed
22 checks with the installed client and the original v0.9.5 server, including
the same CLI reload used to activate the live configuration. Extra desktop
runs lost keyboard focus; input logging confirmed that their failed synthetic
keystrokes did not reach Ghostty.

The original server reports a partial reload because it does not recognize
`next_attention_agent`, `alt+pageup`, or `alt+pagedown`. The newer client loads
these bindings locally and receives the reload notification; all tab shortcuts
still work with that server. No rebuild or server restart is needed.
The live reload preserved server PID 297788, client PID 539319, both workspaces,
all 6 tabs and 15 pane/terminal identities, tab order, and focus.

Smoke-test harnesses are in `.local/tab-reorder-smoke.py` and
`.local/tab-reorder-activate.py`. Verification records are saved under the
configuration backup directory above.

## Change shortcuts later

1. Open `~/.config/herdr/config.toml` and edit the existing `[keys]` section.
2. Assign one shortcut with a string, such as
   `move_tab_next = "ctrl+shift+right"`, or several with an array, such as
   `next_tab = ["prefix+n", "alt+pageup"]`.
3. Use `prefix+...` for a two-step shortcut. Your prefixes are Ctrl+A and Ctrl+B.
4. Remove a shortcut from its old action before assigning it to another. Set
   an action to `""` to disable all its shortcuts.
5. Save, press **prefix → r** (Ctrl+A then r, or Ctrl+B then r) to reload,
   and check **prefix → ?**. No rebuild or server restart is needed.
6. If a shortcut never reaches Herdr, check whether Ghostty or the desktop
   intercepts it.


## Native tab transfer verification (18 November 2026)

The candidate in `/home/odin/herdr-worktrees/native-tab-transfer` builds with
pinned Rust 1.96.1 and Zig 0.16.0. Native Linux `just ci` passed all 4,073 Rust
tests (15 existing manual/profile tests skipped), 150 Python maintenance tests,
UI architecture checks, release workflow tests, and integration assets. The
unreleased API schema, documentation contracts, configuration reference, and
formatting checks passed. `just build` produced `target/release/herdr`.

The release build passed 13 disposable PTY checks: both prefix cancellations,
zoomed three-pane transfer, exact split tree and ratios, labels and agent
state, stable child PIDs and `/dev/pts` descriptors, terminal IDs and history,
refreshed mouse focus, no-op and invalid destination behavior, shell-free new
spaces, focused-terminal current directory, no-focus selection, repeated
caller aliases, unchanged reordering, both picker submission paths, and live
handoff followed by allocation above a retired highest workspace prefix. Three
further checks passed against the original server binary, including feature
gating and continued ordinary typing. The test sessions were stopped and all
test clients closed.

Ghostty 1.3.1 attached to an isolated session and passed the runtime identity,
layout, label, and agent-state checks through the initial transfer. Its
interactive portion could not be validated on this display: the GTK window
remained blank with `eglExportDMABUFImage` failures, and the input log contained
terminal replies but no injected keys or mouse events. Direct window focus,
explicitly addressed events, isolated configuration, and software GTK/GL
settings did not resolve it. This is not counted as a passing interactive
Ghostty smoke; rerun `.local/verify-ghostty.sh` on a working graphical display.
Native UI tests and the PTY mouse/picker checks passed.

The 1- and 15-pane rendering and fanout profiles ran at fixed geometry. With
two clients, steady-state median fanout times for zero versus 4,096 aliases
were 1,349 / 1,352 microseconds at one pane and 1,633 / 1,622 microseconds at
15 panes. The mapping was sent once, with no resend at unchanged revisions.
The general `just bench-render-scale` recipe finished six profiles successfully;
one existing native-file graphics profile failed because this filesystem
cannot supply its required CoW source retention. The feature's render and
relocation-fanout profiles passed; the omnibus benchmark is recorded as
partially blocked, rather than green.

Evidence is retained under the candidate's `.local/`: `ci.log`,
`release-build.log`, `pty-smoke.log`, `legacy-smoke.log`, `ghostty-smoke.log`,
`render-scale.log`, `fanout-profile.log`, and `verification.json`.
`native-tab-transfer.patch` contains only this feature's differences from the
preserved local fork, including its untracked attention-navigation files.
Hash and Git-status checks confirm the original checkout, installed executable,
and personal configuration were preserved. The running personal session was
never addressed; no installation, restart, or commit was performed.
