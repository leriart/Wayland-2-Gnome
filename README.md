# Wayland GNOME Bridge 🏗️

**A protocol-aware Wayland proxy that translates `wlr-layer-shell` calls into GNOME Shell overlays.**

Make any Wayland app that uses `zwlr_layer_shell_v1` work inside GNOME Shell (Mutter) without Mutter implementing the protocol.

## The Problem

GNOME Shell / Mutter explicitly does **not** implement `zwlr_layer_shell_v1`. Apps like `cava-bg`, `rofi`, `eww`, `swaylock`, and many others depend on this protocol for overlays, backgrounds, and panels. On GNOME, they crash with:

```
layer shell not available: the requested global was not found in the registry
```

## Architecture

The bridge sits between the app and the real compositor, selectively intercepting only the protocols it needs to translate:

```
┌──────────────┐   wayland-bridge-0   ┌──────────────────────┐    WAYLAND_DISPLAY    ┌────────────┐
│   App /      │ ◄───────────────────►│ wayland-gnome-bridge │ ◄────────────────────►│  Compositor │
│   Client     │     (fake socket)    │  (sniff & forward)   │    (real socket)     │ (Mutter /   │
│              │                      │                      │                      │  Hyprland)  │
└──────────────┘                      └──────────────────────┘                      └────────────┘
```

### Design

- **Per-client connection**: Each app gets its own thread + compositor connection
- **Sniff-mode global collection**: The bridge forwards `wl_display.get_registry` raw and passively collects global info from the compositor's response
- **Raw passthrough (Phase 4)**: All messages forwarded byte-for-byte. No interception yet
- **Two-step message reading**: Reads exactly one message per `recvmsg()` call (header first, then payload) to avoid consuming trailing data from compositor batches
- **Fully transparent**: Works with any Wayland client, including GPU-accelerated apps via EGL/wgpu

The bridge currently passes the layer shell test (create surface, get layer surface, receive configure) in transparent proxy mode against Hyprland.

### Phase Roadmap

| Phase | Status | Description |
|-------|--------|-------------|
| 1  | ✅ Done | Raw byte-level proxy (passthrough, no parsing) |
| 2  | ✅ Done | Protocol-aware proxy with `wayland_server` dispatchers |
| 3a | ✅ Done | Expose `zwlr_layer_shell_v1` global from bridge |
| 3b | ✅ Done | Intercept `get_layer_surface`, simulate protocol |
| **4** | **✅ Active** | **Transparent proxy with passive sniffing, per-client connections** |
| 5 | 📋 Plan | Intercept `zwlr_layer_shell_v1`, translate to xdg_shell for Mutter |
| 6 | 🧪 Plan | GNOME Shell extension for rendering layer surfaces |

## Key Technical Decisions

### Why sniff-mode instead of separate registry?

**Problem**: Using a bridge-side `get_registry` on the compositor connection creates OID conflicts because the client also sends `get_registry(2)`. Both would share the same compositor socket, leading to two registry objects at OID 2 — a protocol violation.

**Solution**: Forward the client's `get_registry` raw and **sniff** the compositor's global events as they pass through. No separate bridge registry connection needed.

### Why two-step recvmsg?

**Problem**: Wayland compositors batch multiple messages in a single socket write. A single `recvmsg` with a large buffer (65536 bytes) reads ALL queued data, but naive code only processes the first message.

**Solution**: Read 8 bytes (header), extract message size, then read exactly the remaining payload. This guarantees one message per `read_raw()` call.

## Building

```bash
git clone https://github.com/leriart/wayland-gnome-bridge
cd wayland-gnome-bridge
cargo build --release
```

## Usage

```bash
# On Hyprland (or any compositor with zwlr_layer_shell_v1):
RUST_LOG=info WAYLAND_DISPLAY=wayland-1 ./target/release/wayland-gnome-bridge

# In another terminal, run your app through the bridge:
WAYLAND_DISPLAY=wayland-bridge-0 cava-bg
```

The bridge listens on `$XDG_RUNTIME_DIR/wayland-bridge-0` by default and connects to `$WAYLAND_DISPLAY`.

## Current Status

- ✅ **Phase 4**: Transparent proxy working end-to-end
- ✅ All 66 Hyprland globals correctly collected
- ✅ `zwlr_layer_shell_v1` binds forward correctly
- ✅ Layer surface creation and configure events pass through
- ✅ `wl_surface.commit()` triggers compositor configure as expected
- ✅ Session cleanup on client disconnect
- ❌ Layer→xdg protocol translation not yet implemented (Phase 5)

## Related Projects

- [sommelier](https://chromium.googlesource.com/chromiumos/platform2/+/HEAD/vm_tools/sommelier/) — Chrome OS's proxy Wayland compositor (C++)
- [wl-proxy](https://github.com/mahkoh/wl-proxy) — Rust crate for Wayland connection proxying
- [cava-bg](https://github.com/leriart/cava-bg) — Audio visualizer for Wayland, the original motivation

## License

MIT
