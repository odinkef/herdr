# Herdr fork: installation and agent handoff

This guide is for an AI agent installing this local Herdr fork on another
Linux machine similar to the source workstation: Ubuntu 22.04, x86-64,
Ghostty, and a user-local installation in `~/.local/bin`. Rebuild on the target
machine rather than assuming an executable from another architecture or libc
will run there. Read [AGENTS.md](AGENTS.md) before changing repository code.

The upstream base is **Herdr v0.9.5**, commit
`2ecf19a92fc3a7fff75b6ce3655ef04f4625ec01`. The fork remote is
`https://github.com/odinkef/herdr.git`; upstream is
`https://github.com/herdrdev/herdr.git`. The differences below are relative to
that base, not a claim about the latest upstream HEAD.

## 1. Obtain the complete fork source

**Source status on 18 November 2026:** the fork changes are local working-tree
changes, including untracked Rust files. They have not been committed or
pushed as part of this work. A fresh Git clone, `git archive HEAD`, or checking
out the branch name alone will omit these changes.

On the original workstation, there are two checkouts:

| Location | Contents |
| --- | --- |
| `/home/odin/herdr` | Original checkout, branch `attention-navigation`: attention navigation and tab reorder changes. |
| `/home/odin/herdr-worktrees/native-tab-transfer` | Full candidate, branch `native-tab-transfer`: original changes plus native tab transfer, tests, and docs. Use this source for all features below. |

The full candidate was built and tested but **was not installed**. The existing
installation and running personal session were left unchanged. This guide is
present in both checkout roots so it can travel with either source tree.

To transport the complete candidate, run this on the source machine. It copies
the actual files, including untracked source, while excluding Git worktree
pointers, build outputs, and local verification caches:

```bash
HERDR_FORK_SOURCE="$HOME/herdr-worktrees/native-tab-transfer"
test -f "$HERDR_FORK_SOURCE/src/client/shell/attention.rs"
test -f "$HERDR_FORK_SOURCE/src/app/tab_transfer.rs"
test -f "$HERDR_FORK_SOURCE/FORK_SETUP.md"

tar -C "$HERDR_FORK_SOURCE" \
  --exclude='.git' --exclude='./target' --exclude='./.local' \
  --exclude='node_modules' --exclude='.zig-cache' --exclude='zig-out' \
  --exclude='./vendor/libghostty-vt/zig-pkg' \
  -czf "$HOME/herdr-fork-source.tar.gz" .
sha256sum "$HOME/herdr-fork-source.tar.gz"
```

Transfer that archive and its checksum to the target machine. Keep `vendor/`,
`crates/`, `Cargo.lock`, and the hidden `.cargo/` directory. Do not copy just
`src/`. The archive deliberately has no Git history; a Git checkout containing
the committed fork changes is preferable once such a revision is published.
Never copy a worktree's `.git` pointer to a different machine.

On the target, verify the supplied checksum, extract into a new directory,
and inspect the feature sources before building:

```bash
mkdir -p "$HOME/src/herdr-fork"
tar -xzf "$HOME/Downloads/herdr-fork-source.tar.gz" -C "$HOME/src/herdr-fork"
cd "$HOME/src/herdr-fork"
test -f src/client/shell/attention.rs
test -f src/app/tab_transfer.rs
rg -n 'tab\.transfer' src/api/schema.rs
```

If the transfer module is absent, this is the attention-only source. Obtain
the full candidate instead of assuming tab transfer is available. On the
source workstation, a feature-only patch relative to the original local fork
also exists at the candidate's `.local/native-tab-transfer.patch`; that patch
alone does not contain the earlier attention-navigation changes.

## 2. Differences from upstream v0.9.5

| Addition | Behavior |
| --- | --- |
| Attention navigation | `next_attention_agent` cycles blocked agents, then unviewed completions, across the current endpoint/session in workspace/tab/pane order. It is configurable and unbound by default. |
| Client-local attention history | Bindings share one cycle per client. Resolved targets are pruned, new targets join the cycle, and manual navigation or reconnect resets it. Completion visibility remains independent between clients. Sidebar sorting/filtering does not limit the cycle. |
| Extra tab navigation bindings | `pageup` and `pagedown` are accepted key names, including with modifiers. The workstation maps Alt+PageUp to the next tab and Alt+PageDown to the previous tab. Both wrap without reordering. |
| Tab reorder shortcuts | `move_tab_previous` and `move_tab_next` move the current tab one position, keep logical focus, wrap at the ends, and do nothing for a single tab. The workstation binds Ctrl+Shift+Left/Right. These actions are unbound by default. |
| Native whole-tab transfer | `move_tab_to_workspace`, default `prefix+shift+m`, opens **Move to space…**. The same action is in tab context menus, including inactive tabs. The picker searches spaces on the same server, including collapsed groups, and offers **New space…** seeded with the search text. |
| Sidebar drop targets | Drag a tab directly onto a visible space or worktree child. The row highlights; hovering does not switch spaces. Source spaces, other servers, machine headers, and blank sidebar areas are invalid. Tab-bar reordering remains available. |
| Atomic runtime/API support | `tab.transfer` moves the existing complete tab, appends it to its destination, and follows its focused pane by default. It preserves custom labels, exact split tree/ratios, zoom, processes, directories, scrollback, terminal identities, and agent state. |
| Client and handoff identity support | Relocation metadata preserves completion acknowledgements, recency, notifications, and per-client logical locations across moves and reconnects. Handoff carries retired pane aliases and reserves their workspace prefixes. Frozen generation-1 endpoint codecs and the session-file format are unchanged. |

An empty source space is removed without spawning a replacement shell,
closing other worktree spaces, or deleting files/checkouts. A new destination
is an ordinary space whose directory comes from the moved tab's focused
terminal. Transfers stay within one server/session. Picker cancellation,
invalid drops, and stale targets do not submit a transfer. Duplicate submissions
are blocked; uncertain transfers are never retried automatically.

Public tab and pane IDs change when transferred. Processes retain their original
`HERDR_PANE_ID`, `HERDR_TAB_ID`, and `HERDR_WORKSPACE_ID` strings. Retired pane
aliases preserve supported caller addressing; `tab move --current` resolves the
caller's present tab through its pane, rather than trusting inherited tab IDs.
Clients viewing the moved tab follow it; clients viewing unaffected tabs keep
their locations.

```text
herdr tab move (<tab_id> | --current) (--workspace <id> | --new-workspace [--label <name>]) [--focus | --no-focus]
```

Read IDs from actual command output. `--focus` is the transfer default;
`--no-focus` retains an unaffected selection. The JSON API's existing `tab.move`
still means **reorder**; this new CLI command calls **`tab.transfer`**.

The attention and reorder client features work against the original compatible
v0.9.5 server. Native transfers require a server explicitly advertising
`tab.transfer`. A new client attached to an old server stays usable and shows
an unsupported-action notice for transfer.

## 3. Build requirements and verification

For a native Ubuntu/Debian build, install a normal C/C++ build toolchain, Git,
curl, CA certificates, and xz support, for example:

```bash
sudo apt-get update
sudo apt-get install build-essential pkg-config git curl ca-certificates xz-utils python3
```

Install Rust through [rustup](https://rustup.rs/) and the exact toolchain from
[rust-toolchain.toml](rust-toolchain.toml):

```bash
rustup toolchain install 1.96.1 --profile minimal --component clippy --component rustfmt
```

The vendored terminal library requires **Zig 0.16.0**. Obtain its release from
the [official download index](https://ziglang.org/download/index.json), verify
the archive checksum, and unpack it, for example under
`~/.local/opt/zig-0.16.0`. On x86-64 Linux the release archive is
`https://ziglang.org/download/0.16.0/zig-x86_64-linux-0.16.0.tar.xz`, with SHA-256
`70e49664a74374b48b51e6f3fdfbf437f6395d42509050588bd49abe52ba3d00`.
Use the correct architecture entry from the index on other machines.

```bash
export ZIG="$HOME/.local/opt/zig-0.16.0/zig"
"$ZIG" version                 # must print 0.16.0
rustc --version               # repository pin: 1.96.1
```

Building needs network access for uncached Cargo/Zig dependencies. For the
checks, also install `just`, `cargo-nextest`, [Bun](https://bun.sh/)
(CI uses 1.3.14), Python **3.11+**, and a Git version supporting
`git diff --default-prefix` (2.51.3 was used here). Ubuntu 22.04's default Python
and Git are too old for some maintenance tests. Put suitable tools on `PATH`;
do not reuse `/home/odin/...` paths or replace the OS-managed Python. `uv` can
provide an isolated newer Python if available.

```bash
cargo install --locked --version 1.59.0 just
cargo install --locked --version 0.9.151 cargo-nextest
python3 --version
git diff --no-index --default-prefix /dev/null /dev/null

just ci
just docs-contract-test
python3 scripts/config_reference_check.py
just build
./target/release/herdr --version
sha256sum ./target/release/herdr
```

The release executable is `target/release/herdr`, unless `CARGO_TARGET_DIR` is
set. For the commands in this guide, leave that variable unset or adjust the
executable path consistently. Debug builds use the separate `herdr-dev`
configuration/session namespace. `--version` may still print `0.9.5`; version
alone does not distinguish this fork from upstream. Verify source and behavior.

`just check` additionally runs Windows cross-compilation and needs a Windows
SDK/CRT. Native Linux installation uses `just ci` and the checks above; it does
not require that SDK. Do not change frozen fixtures or skip failing checks to
make a new machine appear validated; identify missing tools or real failures.

## 4. Test before replacing an existing installation

Run the candidate in a disposable named session, with its own config/state.
Do this from a normal terminal outside an existing Herdr pane. In the following
example, keep the same shell variables for setup, launch, and cleanup:

```bash
HERDR_FORK_BIN="$PWD/target/release/herdr"
HERDR_TEST_ROOT="$(mktemp -d /tmp/herdr-fork-install.XXXXXX)"
mkdir -p "$HERDR_TEST_ROOT"/{config,state,data,cache,runtime}
chmod 700 "$HERDR_TEST_ROOT/runtime"
printf 'onboarding = false\n' > "$HERDR_TEST_ROOT/test.toml"

herdr_fork_test() {
  env -u HERDR_SOCKET_PATH -u HERDR_CLIENT_SOCKET_PATH -u HERDR_SESSION \
    -u HERDR_PANE_ID -u HERDR_TAB_ID -u HERDR_WORKSPACE_ID \
    XDG_CONFIG_HOME="$HERDR_TEST_ROOT/config" \
    XDG_STATE_HOME="$HERDR_TEST_ROOT/state" \
    XDG_DATA_HOME="$HERDR_TEST_ROOT/data" \
    XDG_CACHE_HOME="$HERDR_TEST_ROOT/cache" \
    XDG_RUNTIME_DIR="$HERDR_TEST_ROOT/runtime" \
    HERDR_CONFIG_PATH="$HERDR_TEST_ROOT/test.toml" \
    "$HERDR_FORK_BIN" --session fork-install-check "$@"
}

herdr_fork_test config check
herdr_fork_test                 # opens the disposable TUI
# After testing, detach with prefix+q, then in the same outer shell:
herdr_fork_test session stop fork-install-check
herdr_fork_test session delete fork-install-check
```

The empty test config uses the default Ctrl+B prefix. To test the workstation
profile below, merge it into the test config before launching. If nesting is
actually needed, enable `[experimental] allow_nested = true` only in this test
config; do not unset `HERDR_ENV` to bypass nesting protection.

Verify both configured prefixes, attention cycling, reload/help, next/previous
tab, reorder/wrap, picker cancellation, inactive-tab context menus, direct
sidebar/worktree drops, new destinations, multi-pane/zoom state, and live output.
Keep the personal session untouched during these checks. The candidate's local
`.local/` harnesses can provide additional evidence when separately copied;
that directory is excluded from the portable source archive above.

## 5. Install and configure the target machine

When installation is authorized, first identify the existing executable with
`type -a herdr` and its running sessions with `herdr session list`. Back up the
binary and config. Do not stop servers simply to replace the executable:
running servers and panes can remain on their old executable until an
intentional upgrade window.

From the tested source directory:

```bash
HERDR_INSTALL_BACKUP="$HOME/.local/share/herdr/backups/fork-$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$HERDR_INSTALL_BACKUP" "$HOME/.local/bin"
if [ -e "$HOME/.local/bin/herdr" ]; then
  cp -pL "$HOME/.local/bin/herdr" "$HERDR_INSTALL_BACKUP/herdr"
fi
if [ -f "$HOME/.config/herdr/config.toml" ]; then
  cp -p "$HOME/.config/herdr/config.toml" "$HERDR_INSTALL_BACKUP/config.toml"
fi
install -m 755 target/release/herdr "$HOME/.local/bin/herdr.new"
mv -f "$HOME/.local/bin/herdr.new" "$HOME/.local/bin/herdr"
export PATH="$HOME/.local/bin:$PATH"
hash -r
command -v herdr
```

If `type -a herdr` identifies another installation location, back that up too
and decide which executable should take precedence. Ensure `~/.local/bin` is
also on the PATH used by desktop launchers, or use its absolute path there.

Merge this profile into the existing `~/.config/herdr/config.toml`. Do not
overwrite unrelated settings or create duplicate TOML tables. These are the
workstation's shortcuts, including the new default transfer action:

```toml
onboarding = false

[keys]
prefix = ["ctrl+a", "ctrl+b"]
next_attention_agent = ["ctrl+alt+a", "prefix+a"]
previous_tab = ["prefix+p", "alt+pagedown"]
next_tab = ["prefix+n", "alt+pageup"]
move_tab_previous = "ctrl+shift+left"
move_tab_next = "ctrl+shift+right"
move_tab_to_workspace = "prefix+shift+m"

split_vertical = ["prefix+|", "prefix+v"]
split_horizontal = "prefix+minus"
focus_pane_left = ["prefix+h", "alt+left", "alt+h"]
focus_pane_down = ["prefix+j", "alt+down", "alt+j"]
focus_pane_up = ["prefix+k", "alt+up", "alt+k"]
focus_pane_right = ["prefix+l", "alt+right", "alt+l"]
resize_pane_left = "alt+shift+left"
resize_pane_down = "alt+shift+down"
resize_pane_up = "alt+shift+up"
resize_pane_right = "alt+shift+right"
resize_mode = "prefix+shift+r"
reload_config = "prefix+r"
rename_pane = "prefix+alt+p"

[ui.toast]
delivery = "system"

[ui.sound]
enabled = true

[theme]
name = "catppuccin"
auto_switch = false
```

Run `herdr config check`. In the TUI use **prefix then r** to reload the local
client settings and selected server settings. An old server may report a
partial reload for fork-only keys; that does not prove the new client failed
to load its local bindings. Use the new client's help (`prefix+?`) and test
the actual shortcuts. Do not remap Ctrl+Shift+Left/Right back to tab switching.

Both Ctrl+A then Shift+M and Ctrl+B then Shift+M open the transfer picker.
`prefix+[` enters copy mode; `v` selects and `y` copies and exits. Ctrl+Alt+A,
Ctrl+A then lowercase a, and Ctrl+B then lowercase a share the attention cycle.

### Server activation and rollback

Replacing a binary or restarting a client does **not** upgrade an already
running server. Native transfer works only after that server runs the candidate.
For a fresh installation, launching the installed `herdr` starts it normally.
For an existing session, save work and arrange a deliberate server upgrade.
`herdr server stop` ends that server's pane processes; persisted layout restore
is not live-process preservation. Do not stop the personal session as a smoke
test. Experimental live handoff is opt-in and should be separately tested with
the actual old/new binary pair before relying on it for important live work.

After an authorized server upgrade, verify an actual transfer on a disposable
tab. A client reporting that `tab.transfer` is unsupported still has an old
selected server. For SSH connections, update the server on that machine too.

To roll back, atomically stage the saved executable back into the selected
installation path, restore only the config changes from this installation,
and reopen/restart the affected client. A server already running the candidate
also needs a deliberate downgrade/restart plan; swapping files does not change
its executable. Re-run the shortcut and ordinary typing checks.

The upstream installer, package upgrades, or `herdr update` can replace the
custom executable with upstream. Keep the fork source and backups; rebuild
this fork to retain its additions.

## 6. Ghostty and the optional F1 dropdown

Herdr does not require Ghostty. On this workstation, Ghostty 1.3.1 forwards
the tab reorder chords with these entries in its existing config (here named
`~/.config/ghostty/config.ghostty`):

```ini
keybind = ctrl+shift+arrow_left=unbind
keybind = ctrl+shift+arrow_right=unbind
```

Merge those entries using the target Ghostty installation's config path, then
reload/reopen it. Desktop-level shortcuts can still consume keys before Herdr
sees them; resolve actual conflicts on the target desktop.

The F1 dropdown is a **separate local desktop integration**, not a Herdr source
change and not included in the source archive above. To reproduce it, transfer
or recreate these files from the source workstation and substitute the target
user's home/executable paths:

| Source file | Purpose |
| --- | --- |
| `~/.local/bin/ghostty-dropdown` | `--prepare` finds the dropdown's WM window, sizes it to the primary monitor's usable work area, makes it sticky/above and hidden from taskbar/pager, then minimizes it. `--toggle` shows/hides that existing window. |
| `~/.config/ghostty/dropdown.conf` | Dedicated undecorated window; launches the absolute path of the installed fork. |
| `~/.config/systemd/user/app-com.mitchellh.ghostty.service` | Keeps Ghostty's GTK application preloaded without an initial window. |
| `~/.config/systemd/user/app-com.mitchellh.ghostty.dropdown.service` | Starts class `com.mitchellh.ghostty.dropdown`, loads `dropdown.conf`, then runs `ghostty-dropdown --prepare`. |

This helper assumes X11 plus a working EWMH window manager. It uses `wmctrl`,
`xdotool`, `xrandr`, `xprop`, `flock`, and Python. A Wayland desktop needs its
compositor's own window rules and shortcuts. Do not hard-code the original
monitor coordinates or `/home/odin` paths. Bind desktop F1 to the target
`ghostty-dropdown --toggle`, rather than creating another terminal on every
keypress. It is optional; validate ordinary Ghostty launching Herdr first.

If these user services are being set up, inspect their contents, import the
target graphical session environment as appropriate, reload the user systemd
manager, and enable/start the selected units. On an existing setup, restarting
only `app-com.mitchellh.ghostty.dropdown.service` restarts the dropdown client,
not the Herdr server. Check F1 show/hide, monitor placement, window properties,
ordinary typing, both prefixes, and all tab shortcuts after changes.

## 7. Verification evidence and limits

The full candidate passed native Linux `just ci`: **4,073 Rust tests**, with
15 existing manual/profile tests skipped; 150 Python maintenance tests;
formatting, Clippy, UI architecture, workflow, and integration-asset checks.
Documentation/configuration/schema checks and `just build` passed. Thirteen
disposable PTY checks passed, including whole-tab identity/scrollback/layout,
agent state, mouse focus, inherited `--current`, no-focus, new-space directory,
and handoff followed by workspace allocation above a retired alias prefix.
Three older-server compatibility checks passed. Relocation fanout at 1 and
15 populated panes showed no growth from 4,096 cached aliases in steady state.

The latest interactive Ghostty smoke was **not fully validated** on the test
display: GTK rendered a blank window with `eglExportDMABUFImage` failures,
and injected keys/clicks did not reach the input log. Runtime transfer checks
with Ghostty attached passed through the initial transfer; PTY input and
deterministic native UI tests passed. Earlier attention/reorder-only Ghostty
checks are historical evidence, not proof of the new transfer UI on a new host.

The general render-scale recipe passed six profiles; one existing native-file
graphics profile requires filesystem CoW source retention and failed on this
filesystem. The feature's rendering/fanout profiles passed separately. Windows
cross-compilation was not part of this native Linux validation. Recheck the
target machine and report these distinctions honestly.

On the source candidate, `.local/verification.json` and the `.local/*.log`
files contain the detailed evidence. These ignored files and `/tmp` smoke
artifacts are machine-local and do not arrive in a clone or the source archive
unless copied separately. More behavior and API details are in
[the local shortcut guide](docs/next/attention-navigation.md),
[keyboard docs](docs/next/website/src/content/docs/keyboard.mdx), and
[socket API docs](docs/next/website/src/content/docs/socket-api.mdx).

For future agents: preserve existing user changes, distinguish client reload
from server replacement, keep smoke tests isolated, and record the source,
toolchain, executable hash, actual server capabilities, checks, and any remaining
limits when handing the installation back to the user.
