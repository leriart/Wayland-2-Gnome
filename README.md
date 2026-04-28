# Wayland 2 GNOME

**A high-performance protocol translation proxy that brings wlr-layer-shell applications to GNOME Shell.**

Wayland 2 GNOME allows applications designed for wlroots-based compositors (like Sway or Hyprland) to run seamlessly on GNOME (Mutter). It translates the `zwlr_layer_shell_v1` protocol into standard XDG surfaces in real-time.

---

## Implemented Features

### 1. Protocol Translation Engine
- **Layer to XDG Mapping**: Translates Layer Shell surfaces into standard XDG Toplevel windows that GNOME understands.
- **XDG Surface Aliasing**: Intercepts secondary `xdg_surface` requests on the same `wl_surface` to prevent protocol violations.
- **Interactive Popups**: Full support for tooltips, context menus, and dropdowns (e.g., Waybar menus) by translating parent-child OID relationships.

### 2. Specialized Window Management
- **Stealth Mode (CSD Forcing)**: Uses `zxdg_decoration_manager_v1` to force Client-Side Decorations, effectively stripping window borders, titles, and shadows for a native "layer" look.
- **Smart Click-through**: Automatically applies empty input regions to Background and Bottom layers, allowing mouse events to pass through to the desktop.
- **HiDPI Support**: Native passthrough of buffer scaling messages, ensuring crisp visuals on 4K and Retina displays.

### 3. Hardware and Output Support
- **Multi-Monitor Aware**: Respects application requests for specific monitor outputs, correctly positioning surfaces across multiple displays.
- **Zero-Copy Performance**: Direct forwarding of DMA-BUF and SHM file descriptors. The bridge does not touch the pixel data, ensuring maximum performance and zero latency.

### 4. Robustness and Integration
- **Dynamic Protocol Injection**: Passive sniffing of the compositor registry. If Layer Shell is missing, the bridge injects a fake global (OID 1000) automatically.
- **Automatic Cleanup**: Efficient garbage collection of Object IDs (OIDs) using `delete_id` monitoring to prevent memory leaks during long sessions.
- **Thread-per-Client**: Scalable architecture where each application runs in its own isolated thread.

---

## Installation

### Prerequisites
- Rust and Cargo (latest stable)
- A Wayland-based GNOME session (Mutter)

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
   *The bridge creates a virtual Wayland display at $XDG_RUNTIME_DIR/wayland-bridge-0.*

2. **Run an application**:
   ```bash
   WAYLAND_DISPLAY=wayland-bridge-0 waybar
   ```

### Logs and Debugging
To monitor the translation process in real-time:
```bash
RUST_LOG=info ./target/release/wayland-2-gnome
```

---

## Technical Status

- [x] Protocol sniffing and global injection
- [x] Layer to XDG surface translation
- [x] Interactive popups and sub-surfaces
- [x] Client-Side Decoration (CSD) forcing
- [x] Empty input regions (Click-through)
- [x] Multi-output targeting
- [x] HiDPI / Buffer scale synchronization
- [x] Efficient OID garbage collection

---

## License

MIT License - Copyright (c) 2024 Leriart
