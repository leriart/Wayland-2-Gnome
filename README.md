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
- [Auto-install Service](#auto-install-service)
- [Multi-monitor](#multi-monitor)
- [Input Handling](#input-handling)
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

### Multi-monitor (Phase 1 -- Dynamic Output Tracking)

- **Live geometry tracking**: The bridge monitors `wl_output.geometry` and `wl_output.scale` events to maintain a map of all connected displays, their positions, and dimensions.
- **Post-collect global detection**: If a new `wl_output` global appears after the initial registry enumeration, the bridge detects it dynamically and forwards it to the client, enabling hotplug support.
- **Coordinate correction**: Output position offsets are used to translate input coordinates between surface-local and compositor-global spaces, so pointer events land correctly on non-primary monitors.

### Input Re-injection (Phase 2 -- Coordinate Offset)

- **wl_pointer.motion offset**: When the bridge runs across multiple monitors, `wl_pointer.motion` coordinates received from the client are offset by the monitor origin before forwarding to the compositor.
- **wl_pointer.enter offset**: Pointer enter events from the compositor are un-offset so the client receives coordinates relative to its own surface.
- **Symmetrical correction**: Both `enter` (op 1) and `motion` (op 0) are corrected bidirectionally, ensuring pointer position accuracy regardless of which monitor the surface is on.

### Auto-install Service (Phase 3)

- **One-command installation**: Run with `--install` to automatically create and enable a systemd user service for the bridge.
- **Persistent setup**: Writes `~/.config/systemd/user/wayland-2-gnome.service` with the current `--socket` and `--compositor` flags baked in, then runs `systemctl --user daemon-reload` and `enable`.
- **Drop-in upgrade**: Re-running with different flags updates the service unit in-place.

### Performance

- **Sniff-and-forward architecture**: Reads raw Wayland wire messages, rewrites relevant object IDs and opcodes, and forwards the rest with minimal overhead.
- **Zero-copy buffer passthrough**: GPU buffers (DMA-BUF) and SHM buffers pass through without re-encoding.
- **calloop event loop**: Session dispatch uses a callback-based reactor for efficient FD readiness handling.

### Reliability

- **Orphan resource cleanup**: When a client disconnects, the bridge destroys all compositor-side objects (xdg_surfaces, toplevels, decorations) to prevent leaks.
- **Daemon mode**: Fork into background with PID file management and signal handlers for graceful shutdown.
- **Configurable limits**: `max_clients` setting caps concurrent sessions.

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
5. **Multi-monitor correction**: `wl_output` geometry events are sniffed to maintain a monitor map. New outputs discovered after collect phase are dynamically forwarded.
6. **Input re-injection**: `wl_pointer.enter` and `wl_pointer.motion` coordinates are offset by the monitor origin before forwarding, and un-offset in the reverse direction.

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

### Install as Systemd Service

```bash
./target/release/wayland-2-gnome --install
```

This writes and enables the service unit, then prints instructions for starting and monitoring it.

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
./target/release/wayland-2-gnome --socket my-bridge-1 --compositor wayland-1
```

Then: `WAYLAND_DISPLAY=my-bridge-1 waybar`

### As a Background Service

```bash
# Install and enable
./target/release/wayland-2-gnome --socket wayland-bridge-0 --compositor wayland-0 --install

# Start
systemctl --user start wayland-2-gnome.service

# View logs
journalctl --user -u wayland-2-gnome.service -f
```

---

## Configuration

### Command-line Flags

| Flag | Description | Default |
| :--- | :--- | :--- |
| `--socket` | Bridge socket name | `wayland-bridge-0` |
| `--compositor` | Compositor socket name | `wayland-0` |
| `--daemon` | Fork into background | false |
| `--config` | Path to TOML configuration file | none |
| `--install` | Install systemd user service and exit | false |

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
- Writes a PID file to `$XDG_RUNTIME_DIR/wayland-bridge-0.pid`.
- Handles SIGTERM, SIGINT, and SIGHUP for graceful shutdown.
- On shutdown, all active sessions are signaled, orphan resources are cleaned up, and the socket file and PID file are removed.

To stop the daemon:

```bash
kill $(cat $XDG_RUNTIME_DIR/wayland-bridge-0.pid)
```

---

## Auto-install Service

```bash
# Install with defaults
./target/release/wayland-2-gnome --install

# Install with custom sockets
./target/release/wayland-2-gnome --socket my-bridge --compositor wayland-1 --install
```

The `--install` flag:
1. Creates `~/.config/systemd/user/` if it does not exist.
2. Writes `wayland-2-gnome.service` with `ExecStart` containing the current `--socket` and `--compositor` values.
3. Runs `systemctl --user daemon-reload`.
4. Runs `systemctl --user enable wayland-2-gnome.service`.
5. Prints post-install instructions and exits.

The bridge does not start when `--install` is used. Run separately:

```bash
systemctl --user start wayland-2-gnome.service
```

---

## Multi-monitor

The bridge tracks all connected outputs by intercepting `wl_output.geometry` (op 4) and `wl_output.scale` (op 6) events. Each output's position, physical dimensions, and scale factor are stored in a `MonitorInfo` map.

When outputs are added or removed at runtime (e.g., monitor hotplug), the bridge:
- Detects new `wl_output` globals in the compositor's registry events after the collect phase.
- Forwards them to the client with proper global name and interface.
- Adds them to the monitor map so input coordinate correction remains accurate.

This is transparent to the client: it sees the correct `wl_output` globals and receives geometry events as expected.

---

## Input Handling

Pointer input received from the client is offset by the first known monitor's origin coordinates before being forwarded to the compositor. This ensures that surfaces not mapped to the primary monitor receive correct compositor-global pointer positions.

In the reverse direction, pointer events from the compositor (both `enter` and `motion`) are un-offset by subtracting the monitor origin, so the client sees coordinates relative to its own surface.

The correction is symmetric and only activates when `monitors.len() > 1`, keeping overhead minimal for single-monitor setups.

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
| Multi-monitor | N/A | Per-output targeting + coordinate offset |
| Dynamic hotplug | N/A | Output global re-injection post-collect |
| Auto-start on login | N/A | systemd --install one-command setup |

### Known Limitations

- Layer surface `exclusive_zone` is not fully translated -- XDG toplevels do not have an equivalent concept. Panel-style surfaces reserve space but may not push other windows as layer surfaces do in wlroots compositors.
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
cli_surface_oid  ->  comp_xdg_surface_oid, comp_toplevel_oid
```

When the client sends messages referencing its layer surface OID, the bridge rewrites them to target the compositor's xdg_surface or toplevel OIDs. In the reverse direction, compositor events targeting the xdg_surface or toplevel are rewritten back to the client's layer surface OID.

### Global Injection

During the collect phase, the bridge reads every `wl_registry.global` event forwarded from the compositor. If no global with interface `"zwlr_layer_shell_v1"` is found, the bridge generates one with a reserved object ID (1000) and sends it to the client after all real globals.

When the client binds to this fake global (OID 1000), the bridge intercepts the `wl_registry.bind` message and does not forward it to the compositor. Instead, the bridge creates a `FakeObject` entry and handles subsequent `get_layer_surface` calls internally.

---

## Development

### Building

```bash
cargo build --release
```

Produces zero warnings with `cargo check`.

### Project Structure

```
src/
  proxy/
    mod.rs             -- Wire protocol, session loop, translation logic
  main.rs              -- Entry point, CLI argument parsing, daemon/service setup
```

## License

MIT
