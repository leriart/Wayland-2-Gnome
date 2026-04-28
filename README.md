# Wayland 2 GNOME

**A high-performance protocol translation proxy that brings wlr-layer-shell applications to GNOME Shell.**

Make tools like cava-bg, waybar, rofi, and swaylock work seamlessly on GNOME (Mutter) without waiting for native protocol support.

---

## The Problem

GNOME's compositor (Mutter) does not implement the zwlr_layer_shell_v1 protocol used by many specialized Wayland desktop components. When running these apps on GNOME, they usually exit with:
`layer shell not available: the requested global was not found in the registry`

## The Solution

**Wayland 2 GNOME** acts as an intelligent intermediary. It intercepts the connection between the app and Mutter, and when it detects a Layer Shell request, it translates it on-the-fly into a standard GNOME-compatible XDG surface.

### Key Capabilities

*   **Zero-Latency Proxy**: Forwards GPU buffers (DMA-BUF) and shared memory (SHM) directly, ensuring no performance penalty.
*   **Intelligent Translation**: Maps LayerSurface requests to xdg_toplevel windows.
*   **Stealth Mode**: Automatically forces Client-Side Decorations (CSD) to hide window titles and borders, making apps look like true system layers.
*   **Input Passthrough**: Automatically configures empty input regions for background layers so mouse clicks pass through to the desktop.
*   **Multi-Monitor Aware**: Respects application requests for specific monitors/outputs.
*   **Dynamic Injection**: Only activates when needed; if the compositor already supports Layer Shell, it can stay in passive mode.

---

## Architecture

```
┌──────────────┐      wayland-bridge-0      ┌──────────────────────┐      WAYLAND_DISPLAY     ┌────────────┐
│   Client     │ <────────────────────────> │   Wayland 2 GNOME    │ <──────────────────────> │  Compositor │
│ (e.g. Waybar)│      (Fake Registry)       │  (Translation Logic) │      (Real Socket)       │  (Mutter)   │
└──────────────┘                            └──────────────────────┘                          └────────────┘
```

The bridge manages a complex 1-to-N mapping:
- **1 Layer Surface** (App perspective) -> **1 XDG Surface + 1 XDG Toplevel + 1 Decoration Manager** (Compositor perspective).

---

## Installation

### Prerequisites
- Rust and Cargo (latest stable)
- A Wayland-based GNOME session

### Build
```bash
git clone https://github.com/leriart/Wayland-2-Gnome
cd Wayland-2-Gnome
cargo build --release
```

The binary will be available at `./target/release/wayland-2-gnome`.

---

## Usage

1. **Start the bridge**:
   ```bash
   ./target/release/wayland-2-gnome
   ```
   *By default, it creates a new Wayland socket at $XDG_RUNTIME_DIR/wayland-bridge-0.*

2. **Run your app**:
   ```bash
   WAYLAND_DISPLAY=wayland-bridge-0 cava-bg
   ```

### Debugging
You can see the translation in real-time by enabling logs:
```bash
RUST_LOG=info ./target/release/wayland-2-gnome
```

---

## Comparison

| Feature | Native GNOME | With Bridge |
| :--- | :---: | :---: |
| xdg_shell apps | Yes | Yes |
| wlr-layer-shell apps | No (Crash) | Yes (Working) |
| Transparent Overlays | No | Yes |
| Click-through Backgrounds | No | Yes |
| Borders & Titles | Forced | Hidden |

---

## Roadmap and Status

- [x] **Phase 1-4**: High-speed byte-level proxying and global sniffing.
- [x] **Phase 5**: Layer-to-XDG protocol translation core.
- [x] **Phase 6**: Advanced window management (Decorations and Alpha).
- [x] **Multi-Monitor**: Support for specific output targeting.
- [x] **Garbage Collection**: Efficient OID cleanup on client disconnect.
- [ ] **Interactive Popups**: Full support for sub-menus and tooltips.

## Contributing

Contributions are welcome! If you find a specific app that doesn't behave correctly through the bridge, please open an issue with the RUST_LOG=debug output.

## License

MIT License - Copyright (c) 2024 Leriart
