#!/usr/bin/env bash
set -euo pipefail

# ── Build Release Binary ──────────────────────────────────────────────────────

PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$PROJECT_DIR"

echo "==> Building release binary..."
cargo build --release

BINARY="target/release/wayland-2-gnome"
echo "    Binary size: $(wc -c < "$BINARY") bytes"
echo "    Binary path: $BINARY"

# ── Create Distribution Archives ─────────────────────────────────────────────

VERSION="0.1.0"
ARCH="$(uname -m)"
ARCHIVE_NAME="wayland-2-gnome-${VERSION}-linux-${ARCH}"

DIST_DIR="target/dist/${ARCHIVE_NAME}"
mkdir -p "$DIST_DIR"

# Copy binary
cp "$BINARY" "$DIST_DIR/"

# Copy documentation
cp README.md LICENSE "$DIST_DIR/"

# Create a sample config
cat > "$DIST_DIR/config.toml" << 'TOML'
# Wayland 2 GNOME — sample configuration
# Place this at ~/.config/wayland-2-gnome/config.toml

# Bridge socket name (under $XDG_RUNTIME_DIR)
bridge_display = "wayland-bridge-0"

# Compositor socket name (under $XDG_RUNTIME_DIR)
compositor_display = "wayland-0"

# Maximum concurrent client sessions (0 = unlimited)
max_clients = 10

# Log level: error, warn, info, debug, trace
log_level = "info"
TOML

# Create systemd user service file
mkdir -p "$DIST_DIR/systemd"
cat > "$DIST_DIR/systemd/wayland-2-gnome.service" << 'SERVICE'
[Unit]
Description=Wayland 2 GNOME — protocol translation proxy for wlr-layer-shell
Documentation=https://github.com/leriart/Wayland-2-Gnome
After=graphical-session.target
PartOf=graphical-session.target

[Service]
Type=exec
ExecStart=%h/.local/bin/wayland-2-gnome --daemon
ExecStop=/bin/kill $MAINPID
Restart=on-failure
RestartSec=5
Environment=WAYLAND_DISPLAY=wayland-0
Environment=XDG_RUNTIME_DIR=%t

[Install]
WantedBy=graphical-session.target
SERVICE

# Create autostart .desktop entry
mkdir -p "$DIST_DIR/autostart"
cat > "$DIST_DIR/autostart/wayland-2-gnome.desktop" << 'DESKTOP'
[Desktop Entry]
Type=Application
Name=Wayland 2 GNOME
Comment=Protocol translation proxy for wlr-layer-shell applications
Exec=%h/.local/bin/wayland-2-gnome --daemon
Terminal=false
StartupNotify=false
X-GNOME-Autostart-enabled=true
X-GNOME-Autostart-Delay=3
Categories=Utility;
DESKTOP

# ── Create archives ──────────────────────────────────────────────────────────

cd "target/dist"

# tar.gz
echo "==> Creating ${ARCHIVE_NAME}.tar.gz..."
tar czf "${ARCHIVE_NAME}.tar.gz" "$ARCHIVE_NAME/"

# sha256sum
echo "==> Generating checksums..."
sha256sum "${ARCHIVE_NAME}.tar.gz" > "${ARCHIVE_NAME}.tar.gz.sha256"

echo ""
echo "    Archive: target/dist/${ARCHIVE_NAME}.tar.gz"
echo "    Checksum: target/dist/${ARCHIVE_NAME}.tar.gz.sha256"
echo ""
echo "==> Done!"
