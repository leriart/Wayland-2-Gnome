#!/usr/bin/env bash
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$PROJECT_DIR"

echo "==> Building release binary..."
cargo build --release

BINARY="target/release/wayland-2-gnome"
echo "    Binary size: $(wc -c < "$BINARY") bytes"

VERSION="$(grep '^version =' Cargo.toml | head -1 | cut -d'"' -f2)"
ARCH="$(uname -m)"
ARCHIVE_NAME="wayland-2-gnome-${VERSION}-linux-${ARCH}"

DIST_DIR="target/dist/${ARCHIVE_NAME}"
rm -rf "$DIST_DIR" "target/dist/${ARCHIVE_NAME}.tar.gz" "target/dist/${ARCHIVE_NAME}.tar.gz.sha256"
mkdir -p "$DIST_DIR"

# Binary
cp "$BINARY" "$DIST_DIR/"

# Documentation
cp README.md LICENSE "$DIST_DIR/"

# Sample config
cat > "$DIST_DIR/config.toml" << 'TOML'
# Wayland 2 GNOME — sample configuration
# Place at ~/.config/wayland-2-gnome/config.toml

bridge_display = "wayland-bridge-0"
compositor_display = "wayland-0"
max_clients = 10
log_level = "info"
TOML

# Systemd user service
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
Environment=XDG_RUNTIME_DIR=%t

[Install]
WantedBy=graphical-session.target
SERVICE

# GNOME autostart
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

# Create archives
cd "target/dist"

echo "==> Creating ${ARCHIVE_NAME}.tar.gz..."
tar czf "${ARCHIVE_NAME}.tar.gz" "$ARCHIVE_NAME/"

echo "==> Generating checksums..."
sha256sum "${ARCHIVE_NAME}.tar.gz" > "${ARCHIVE_NAME}.tar.gz.sha256"

echo ""
echo "    Archive: target/dist/${ARCHIVE_NAME}.tar.gz"
echo "    Checksum: target/dist/${ARCHIVE_NAME}.tar.gz.sha256"
echo "    Binary version: v${VERSION}"
echo ""
echo "==> Done!"
