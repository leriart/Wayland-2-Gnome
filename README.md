# Wayland GNOME Bridge 🏗️

**A protocol-aware Wayland proxy that translates `wlr-layer-shell` calls into GNOME Shell overlays.**

Make any Wayland app that uses `zwlr_layer_shell_v1` work inside GNOME Shell (Mutter) without Mutter implementing the protocol.

## The Problem

GNOME Shell / Mutter explicitly does **not** implement `zwlr_layer_shell_v1`. Apps like `cava-bg`, `rofi`, `eww`, `swaylock`, and many others depend on this protocol for overlays, backgrounds, and panels. On GNOME, they crash with:

```
layer shell not available: the requested global was not found in the registry
```

## The Solution

This project sits as a **Wayland proxy** between the app and GNOME Mutter:

```
┌──────────────┐      Wayland socket       ┌──────────────────┐      real Wayland socket       ┌────────────┐
│ App (cava-bg)│ ◄─────────────────────────►│ wayland-gnome-bridge │ ◄────────────────────────────►│ GNOME      │
│              │      zwlr_layer_shell      │                  │      xdg_shell + extensions    │ Mutter     │
│              │      wl_compositor         │  Proxy Server    │      wl_compositor              │ Compositor │
└──────────────┘      wl_shm etc.           └──────────────────┘      wl_shm etc.                 └────────────┘
```

### How it works

1. **Starts a fake Wayland compositor** on a custom socket (e.g., `$XDG_RUNTIME_DIR/wayland-gnome-bridge-0`)
2. **Connects to the real GNOME Mutter** on the actual `$WAYLAND_DISPLAY`
3. **Acts as a transparent proxy** for all standard Wayland protocols (`wl_compositor`, `wl_shm`, `wl_subcompositor`, etc.)
4. **Intercepts `zwlr_layer_shell_v1`** calls and translates them into:
   - **Background layer** → xdg_shell window positioned behind (rendered to a texture via offscreen wgpu/EGL, composited by the GNOME extension)
   - **Overlay / Panel layer** → GNOME Shell extension that paints the surface as an on-screen `St.ImageContent`
5. **Forwards input events** (mouse, keyboard) from GNOME back to the proxied app

## Architecture

```
wayland-gnome-bridge/
├── src/
│   ├── main.rs              # Entry point: socket setup, daemon mode
│   ├── proxy/
│   │   ├── mod.rs            # Core proxy loop: read from client → dispatch → forward to compositor
│   │   ├── connection.rs     # Client & compositor socket management
│   │   ├── registry.rs       # Global registry: filter/modify globals exposed to client
│   │   └── object_map.rs     # Maps object IDs between client and compositor sides
│   ├── translate/
│   │   ├── mod.rs            # Translation dispatcher
│   │   ├── layer_shell.rs    # zwlr_layer_shell_v1 → xdg_shell + compositor calls
│   │   └── xdg_shell.rs      # xdg_shell passthrough helpers
│   ├── output/
│   │   ├── mod.rs            # Output management (virtual outputs for layers)
│   │   └── capture.rs        # Frame capture → pipe to GNOME extension
│   └── gnome_extension/      # The GNOME Shell extension side
│       ├── extension.js      # Receives frames via socket, paints as overlay
│       ├── metadata.json
│       └── schemas/          # GSettings for configuration
├── Cargo.toml
└── README.md
```

## Phase 1: Minimal Viable Proxy (MVP)

The first milestone is a **raw passthrough proxy** that:
- Listens on a custom Unix socket
- Connects to GNOME Mutter
- Forwards every message transparently (byte-level passthrough)
- Runs an app transparently through it (same behavior as direct connection)

This proves the proxying mechanism works.

## Phase 2: Protocol Interception

- Intercept `wl_display.get_registry` → modify/filter globals exposed
- Intercept `zwlr_layer_shell_v1` → intercept all its objects and requests
- For all other globals: transparent passthrough

## Phase 3: Layer → Surface Translation

- Translate `zwlr_layer_shell_v1.get_layer_surface` into an xdg_toplevel with special properties
- Handle:
  - Anchor mappings (top, bottom, left, right, center)
  - Layer stacking (background, bottom, top, overlay)
  - Exclusive zones
  - Keyboard interactivity
- Surface the app's wl_surface as an xdg_surface under GNOME

## Phase 4: GNOME Shell Extension

- Unix socket listener in GJS that receives the app's rendered frames
- `St.ImageContent` + `Clutter.Actor` overlay painting
- Multi-monitor support
- Click-through mode (reactive: false)

## Phase 5: Polish

- Config file (`~/.config/wayland-gnome-bridge/config.toml`)
- Auto-detection of running compositor
- `systemd --user` service integration
- App-specific rules (which layers go where)

## How to Contribute

This is a complex project that touches:
- **Rust** (Wayland protocol, wayland-server, wl-proxy)
- **GJS** (GNOME Shell Extension)
- **Wayland internals** (wl_registry, object IDs, protocol marshalling)
- **wgpu/EGL** (offscreen rendering)

Check the [issues](https://github.com/leriart/wayland-gnome-bridge/issues) for open tasks.

## Related Projects

- [sommelier](https://chromium.googlesource.com/chromiumos/platform2/+/HEAD/vm_tools/sommelier/) — Chrome OS's proxy Wayland compositor (C++)
- [wl-proxy](https://github.com/mahkoh/wl-proxy) — Rust crate for Wayland connection proxying
- [cava-bg](https://github.com/leriart/cava-bg) — Audio visualizer for Wayland, the original motivation

## License

MIT
