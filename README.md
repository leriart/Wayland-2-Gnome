# Wayland 2 GNOME

**A high-performance protocol translation proxy that bridges the gap between wlr-layer-shell and GNOME Shell.**

Wayland 2 GNOME acts as a protocol-aware layer between specialized Wayland apps and the GNOME compositor (Mutter). It enables applications designed for tiling window managers (like Sway or Hyprland) to run natively on GNOME by translating incompatible protocols into standard XDG surfaces in real-time.

---

## Visual Demo

### Desktop Integration
The bridge allows background visualizers and panels to blend perfectly with the GNOME desktop environment.

### Interface Translation
Complex panels and launchers are rendered without window borders, maintaining their original design intent.


### Interactive Overlays
Full support for popups and menus, ensuring tooltips and dropdowns function as expected.

---

## Key Features

### Protocol Translation Engine
*   **Layer to XDG Mapping**: Seamlessly converts LayerSurface requests into standard windows.
*   **Interactive Popups**: Full support for tooltips, context menus, and dropdowns.
*   **XDG Aliasing**: Intelligently handles applications that mix Layer Shell and XDG Shell protocols.

### Visual and Desktop Integration
*   **Stealth Mode**: Forces Client-Side Decorations (CSD) to remove window borders, shadows, and titles.
*   **Smart Click-through**: Automatically makes Background and Bottom layers invisible to mouse clicks.
*   **HiDPI Perfect**: Native passthrough for buffer scaling, ensuring crisp visuals on 4K/Retina displays.

### Performance and Reliability
*   **Zero-Copy Proxy**: Direct forwarding of GPU buffers (DMA-BUF) and SHM. No added latency.
*   **Multi-Monitor Support**: Correctly targets specific outputs/monitors as requested.
*   **Resource Management**: Active garbage collection of Object IDs (OIDs) to prevent memory leaks.

---

## Architecture

```text
┌────────────────┐      wayland-bridge-0      ┌────────────────────┐      WAYLAND_DISPLAY     ┌──────────────┐
│   Your App     │ <────────────────────────> │  Wayland 2 GNOME   │ <────────────────────────> │ GNOME Mutter │
│ (Waybar, Rofi) │      (Fake Registry)       │ (Translation Core) │       (Real Socket)        │ (Compositor) │
└────────────────┘                              └────────────────────┘                            └──────────────┘
```

---

## Installation

### Prerequisites
*   Rust (latest stable)
*   GNOME Shell (Wayland session)

### Build from Source
```bash
git clone https://github.com/leriart/Wayland-2-Gnome
cd Wayland-2-Gnome
cargo build --release
```

### Running the Bridge
1.  **Launch the translator**:
    ```bash
    ./target/release/wayland-2-gnome
    ```

2.  **Run an application**:
    ```bash
    WAYLAND_DISPLAY=wayland-bridge-0 waybar
    ```

---

## Compatibility Matrix

| Feature | Without Wayland 2 GNOME | With Wayland 2 GNOME |
| :--- | :---: | :---: |
| **wlr-layer-shell Apps** | Crash on startup | Runs Seamlessly |
| **Window Decorations** | Forced Titles/Borders | Clean and Borderless |
| **Mouse Interaction** | Blocks Desktop | Click-through Support |
| **Monitor Targeting** | Ignored | Output-aware Positioning |
| **Performance** | N/A | Zero Latency |

---

## Technical Details

The bridge implements a **Sniff-and-Forward** architecture. Instead of creating a separate registry connection, it passively monitors the compositor's response to the client's registry requests.

If the compositor lacks zwlr_layer_shell_v1, the bridge:
1.  Injects a Fake Global (OID 1000).
2.  Intercepts Binds to that global.
3.  Translates requests into a multi-object XDG stack (Surface, Toplevel, and Decoration).
4.  Rewrites Event IDs in the compositor-to-client direction to maintain consistency.

---

## License

This project is licensed under the MIT License.
