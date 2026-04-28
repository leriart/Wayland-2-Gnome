# Wayland 2 GNOME

A protocol translation proxy that bridges wlr-layer-shell applications and GNOME Shell. It translates incompatible Wayland protocols into standard XDG surfaces in real time, letting apps built for tiling window managers run natively on GNOME.

---

## Table of Contents

- [Overview](#overview)
- [Features](#features)
- [Architecture](#architecture)
- [Installation](#installation)
- [Usage](#usage)
- [Configuration](#configuration)
- [Daemon Mode](#daemon-mode)
- [Compatibility Matrix](#compatibility-matrix)
- [How It Works](#how-it-works)
- [Development](#development)
- [License](#license)

---

## Overview

Applications like Waybar, Rofi, and nwg-dock use the `zwlr_layer_shell_v1` protocol to create panels, launchers, and overlays that blend into the desktop. GNOME Shell (Mutter) does not implement this protocol, so those apps fail on stock GNOME.

Wayland 2 GNOME sits between the application and Mutter, intercepting the session at the wire level. When it detects that the compositor lacks `zwlr_layer_shell_v1`, it injects a fake global, intercepts binds, and translates every `get_layer_surface` call into a standard XDG stack: `xdg_surface` + `xdg_toplevel` + `zxdg_decoration_manager_v1`. The app sees its expected protocol, Mutter sees standard windows, and both are happy.

---

## Features

### Protocol Translation

- **Layer to XDG mapping**: Converts `zwlr_layer_shell_v1.get_layer_surface` into `xdg_wm_base.get_xdg_surface` + `xdg_surface.get_toplevel` + decoration requests.
- **Interactive popups**: Tooltips, context menus, and dropdowns work through proper anchor/position translation.
- **Global injection**: When the compositor lacks `zwlr_layer_shell_v1`, a fake global is injected into the client's registry. The client binds to the fake ID, and the bridge transparently manages the mismatch between client-facing and compositor-facing object IDs.

### Desktop Integration

- **Borderless rendering**: Forces Client-Side Decorations (CSD) to remove window borders, shadows, and title bars from translated surfaces.
- **Click-through**: Background and Bottom layer surfaces are rendered invisible to mouse input so the desktop remains interactive.
- **Output-aware positioning**: Translates layer surface monitor targets into proper XDG output tracking for multi-monitor setups.

### Performance

- **Sniff-and-forward architecture**: Reads raw Wayland wire messages, rewrites relevant object IDs and opcodes, and forwards the rest with minimal overhead.
- **Zero-copy buffer passthrough**: GPU buffers (DMA-BUF) and SHM buffers pass through without re-encoding.
- **calloop event loop**: Session dispatch uses a callback-based reactor for efficient FD readiness handling.

### Reliability

- **Orphan resource cleanup**: When a client disconnects, the bridge destroys all compositor-side objects (xdg_surfaces, toplevels, decorations) to prevent leaks.
- **Daemon mode**: Fork into background with PID file management and signal handlers for graceful shutdown.
- **Configurable limits**: `max_clients` setting caps concurrent sessions.
- **End-to-end tests**: Integration tests verify registry forwarding and layer surface translation against a mock compositor.

---

## Architecture

```
+------------------+     wayland-bridge-0     +------------------------+     WAYLAND_DISPLAY     +------------------+
|   Your App       | <--------------------->  |   Wayland 2 GNOME     | <--------------------->  |  GNOME Mutter    |
| (Waybar, Rofi,   |    (fake registry)       |   (translation core)  |      (real socket)      |  (compositor)    |
|  nwg-dock)       |                          |                        |                          |                  |
+------------------+                          +------------------------+                          +------------------+
```

The bridge binds a Unix domain socket (default: `wayland-bridge-0`) under `$XDG_RUNTIME_DIR`. Applications connect to this socket instead of the real compositor. The bridge maintains a single connection to Mutter and multiplexes messages with object ID rewriting.

### Translation Pipeline

1. **Collect phase**: The bridge reads registry globals from the compositor and forwards them to the client. If `zwlr_layer_shell_v1` is absent, it injects the missing global after all real globals have been advertised.
2. **Bind interception**: When the client binds to the fake layer shell global, the bridge tracks the object and lets the compositor assign a real ID.
3. **Request translation**: `get_layer_surface` calls are rewritten as `get_xdg_surface` + `get_toplevel` with layer-specific properties (anchor, margin, size) discarded or mapped to XDG equivalents.
4. **Event forwarding**: Compositor events are forwarded to the client with ID remapping so the client sees its expected objects.

---

## Installation

### Prerequisites

- Rust (latest stable, edition 2021)
- GNOME Shell running on a Wayland session

### Build from Source

```bash
git clone https://github.com/leriart/Wayland-2-Gnome
cd Wayland-2-Gnome
cargo build --release
```

The binary is placed at `target/release/wayland-2-gnome`.

---

## Usage

### Basic

```bash
./target/release/wayland-2-gnome
```

This starts the bridge listening on `$XDG_RUNTIME_DIR/wayland-bridge-0`, proxying to the compositor on `$XDG_RUNTIME_DIR/wayland-0`.

### Running an App Through the Bridge

```bash
WAYLAND_DISPLAY=wayland-bridge-0 waybar
WAYLAND_DISPLAY=wayland-bridge-0 rofi -show run
WAYLAND_DISPLAY=wayland-bridge-0 nwg-dock
```

### Custom Display Names

```bash
./target/release/wayland-2-gnome --bridge-display my-bridge-1 --compositor-display wayland-1
```

Then: `WAYLAND_DISPLAY=my-bridge-1 waybar`

---

## Configuration

### Command-line Flags

| Flag | Description | Default |
| :--- | :--- | :--- |
| `--bridge-display` | Bridge socket name | `wayland-bridge-0` |
| `--compositor-display` | Compositor socket name | `wayland-0` |
| `--max-clients` | Maximum concurrent sessions | 0 (no limit) |
| `--log-level` | Log level (error, warn, info, debug, trace) | `info` |
| `--daemon` | Fork into background | false |
| `--config` | Path to TOML configuration file | none |
| `--pid-file` | PID file path (daemon mode) | `$XDG_RUNTIME_DIR/wayland-2-gnome.pid` |

### TOML Configuration File

Settings are merged hierarchically: defaults are overridden by file values, which are overridden by CLI flags.

```toml
# ~/.config/wayland-2-gnome/config.toml
bridge_display = "wayland-bridge-0"
compositor_display = "wayland-0"
max_clients = 10
log_level = "debug"
```

```bash
./target/release/wayland-2-gnome --config ~/.config/wayland-2-gnome/config.toml
```

---

## Daemon Mode

```bash
./target/release/wayland-2-gnome --daemon
```

- Forks into the background with `setsid()`.
- Writes a PID file to `$XDG_RUNTIME_DIR/wayland-2-gnome.pid`.
- Handles SIGTERM, SIGINT, and SIGHUP for graceful shutdown.
- On shutdown, all active sessions are signaled, orphan resources are cleaned up, and the socket file and PID file are removed.

To stop the daemon:

```bash
kill $(cat $XDG_RUNTIME_DIR/wayland-2-gnome.pid)
```

---

## Compatibility Matrix

| Feature | Without Wayland 2 GNOME | With Wayland 2 GNOME |
| :--- | :---: | :---: |
| wlr-layer-shell apps | Crash on startup | Run seamlessly |
| Window decorations | Forced title bars and borders | Clean, borderless surfaces |
| Mouse interaction | Blocks desktop interaction | Click-through on background layers |
| Monitor targeting | Ignored | Output-aware positioning |
| Interactive popups | Not available | Tooltips, menus, dropdowns |
| HiDPI (buffer scale) | N/A | Native passthrough |
| Multi-monitor | N/A | Per-output targeting |

### Known Limitations

- Layer surface `exclusive_zone` is not fully translated — XDG toplevels do not have an equivalent concept. Panel-style surfaces reserve space but may not push other windows as layer surfaces do in wlroots compositors.
- `zwlr_layer_surface_v1.set_keyboard_interactivity` has limited XDG equivalents; keyboard focus behavior may differ.
- Animations and transitions specific to layer surfaces (slide-in, fade) are not reproduced on XDG surfaces.

---

## How It Works

### Sniff-and-Forward Architecture

Unlike protocol-level proxy libraries, Wayland 2 GNOME operates at the Wayland wire format level. It reads raw byte messages (`recvmsg`/`sendmsg`), parses the 8-byte header (object ID, opcode, size), and makes routing decisions based on the object-opcode pair.

This approach:
- Avoids linking against wayland-server or wayland-client for the proxy path.
- Allows transparent forwarding of unknown protocols and future extensions.
- Keeps the hot path minimal: most messages pass through with only ID remapping.

### Object ID Translation

Each session maintains a `TranslationEntry` for every layer surface that has been translated. The entry maps:

```
cli_surface_oid  ->  comp_xdg_surface_oid, comp_toplevel_oid, comp_decoration_oid
```

When the client sends messages referencing its layer surface OID, the bridge rewrites them to target the compositor's xdg_surface or toplevel OIDs. In the reverse direction, compositor events targeting the xdg_surface or toplevel are rewritten back to the client's layer surface OID.

### Global Injection

During the collect phase, the bridge reads every `wl_registry.global` event forwarded from the compositor. If no global with interface `"zwlr_layer_shell_v1"` is found, the bridge generates one with a reserved object ID (1000) and sends it to the client after all real globals.

When the client binds to this fake global (OID 1000), the bridge intercepts the `wl_registry.bind` message and does not forward it to the compositor. Instead, the bridge creates a `FakeObject` entry and handles subsequent `get_layer_surface` calls internally.

---

## Development

### Running Tests

```bash
cargo test
```

Two integration tests validate the core translation pipeline against a mock compositor:

- `test_registry_forwarding_and_layer_shell_injection` -- verifies that registry globals are forwarded, and that `zwlr_layer_shell_v1` is injected when absent.
- `test_get_layer_surface_translation` -- verifies that a client's `get_layer_surface` call is translated into the correct XDG surface + toplevel creation sequence on the compositor side.

### Project Structure

```
src/
  main.rs              -- Entry point, CLI argument parsing, daemon setup
  lib.rs               -- Library crate root, re-exports for testing
  proxy/
    mod.rs             -- Wire protocol, session loop, translation logic
tests/
  integration_test.rs  -- End-to-end tests with mock compositor
```

### Code Quality

- All warnings are eliminated (`cargo check` produces zero warnings).
- `#[allow(dead_code)]` attributes are used sparingly for future-proofing (reserved functions, struct fields).
- Dual crate structure: `src/lib.rs` defines the library crate, `src/main.rs` is the binary, enabling external integration tests.

---

## License

MIT
