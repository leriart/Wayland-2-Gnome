# Wayland 2 GNOME release checklist and artifacts.

---

# Release Checklist

Use this checklist when cutting a new release. Run the steps in order.

## 1. Update Version

Edit `Cargo.toml`:

```toml
version = "0.2.0"   # bump as needed
```

Follow semver:
- **Patch** (0.1.0 -> 0.1.1): bug fixes, minor improvements.
- **Minor** (0.1.0 -> 0.2.0): new features, protocol changes.
- **Major** (0.1.0 -> 1.0.0): stable API, breaking changes.

## 2. Update Changelog

Add a new section at the top of `CHANGELOG.md` (create if missing):

```markdown
## [0.1.0] — 2026-04-28

### Added
- Sniff-and-forward proxy with raw Wayland wire protocol.
- `zwlr_layer_shell_v1` global injection when absent from compositor.
- Full translation: get_layer_surface -> xdg_surface + xdg_toplevel + decoration.
- Interactive popups with anchor/position translation.
- Output-aware multi-monitor targeting.
- Orphan resource cleanup on client disconnect.
- Daemon mode with PID file and signal handlers.
- TOML configuration file with hierarchical merge (default -> file -> CLI).
- calloop event loop replacing raw libc::poll.
- End-to-end integration tests with mock compositor.

### Changed
- Session loop migrated from libc::poll to calloop EventLoop.
- Accept loop uses nonblocking socket + sleep pattern.

### Fixed
- Dead-code warnings (5 warnings -> 0).
- Collect phase fall-through causing session deadlock.
```

## 3. Run Tests

```bash
cargo test          # all tests pass
cargo check         # zero warnings
cargo build --release
```

## 4. Create GitHub Release

```bash
./release/build.sh
```

This creates:
- `target/dist/wayland-2-gnome-0.1.0-linux-x86_64.tar.gz`
- `target/dist/wayland-2-gnome-0.1.0-linux-x86_64.tar.gz.sha256`

## 5. Publish Release on GitHub

1. Go to https://github.com/leriart/Wayland-2-Gnome/releases
2. Click "Create a new release"
3. Tag: `v0.1.0`
4. Title: `v0.1.0`
5. Paste changelog section as description
6. Attach:
   - `wayland-2-gnome-0.1.0-linux-x86_64.tar.gz`
   - `wayland-2-gnome-0.1.0-linux-x86_64.tar.gz.sha256`
7. Click "Publish release"

---

# Artifacts

The `release/build.sh` script produces these files:

## Archive Contents

```
wayland-2-gnome-0.1.0-linux-x86_64/
  wayland-2-gnome           # static binary (~2 MB)
  README.md                 # documentation
  LICENSE                   # MIT license
  config.toml               # sample configuration file
  systemd/
    wayland-2-gnome.service # user service unit
  autostart/
    wayland-2-gnome.desktop # GNOME autostart entry
```

## Binary Info

| Property | Value |
| :--- | :--- |
| Format | Static binary (stripped) |
| Size | ~2 MB |
| Rust edition | 2021 |
| Architecture | x86_64 (aarch64 builds available on request) |
| Dependencies | None runtime — links against system libc only |

---

# Installation Methods

## Method 1: Manual Install (all distros)

```bash
# Download and extract
curl -LO https://github.com/leriart/Wayland-2-Gnome/releases/download/v0.1.0/wayland-2-gnome-0.1.0-linux-x86_64.tar.gz
tar xzf wayland-2-gnome-0.1.0-linux-x86_64.tar.gz

# Install binary
mkdir -p ~/.local/bin
cp wayland-2-gnome-0.1.0-linux-x86_64/wayland-2-gnome ~/.local/bin/

# Install autostart (GNOME only)
mkdir -p ~/.config/autostart
cp wayland-2-gnome-0.1.0-linux-x86_64/autostart/wayland-2-gnome.desktop ~/.config/autostart/

# Log out and back in, or start manually
wayland-2-gnome
```

## Method 2: Systemd User Service (systemd distros)

```bash
# Install binary
cp wayland-2-gnome-0.1.0-linux-x86_64/wayland-2-gnome ~/.local/bin/

# Install systemd user service
mkdir -p ~/.config/systemd/user
cp wayland-2-gnome-0.1.0-linux-x86_64/systemd/wayland-2-gnome.service ~/.config/systemd/user/

# Enable and start
systemctl --user daemon-reload
systemctl --user enable --now wayland-2-gnome

# Check status
systemctl --user status wayland-2-gnome
```

## Method 3: Build from Source (recommended for packagers)

```bash
git clone https://github.com/leriart/Wayland-2-Gnome
cd Wayland-2-Gnome
cargo build --release
sudo cp target/release/wayland-2-gnome /usr/local/bin/
```

---

# Distro-Specific Packaging Notes

## Arch Linux (PKGBUILD)

A PKGBUILD template is provided in `release/pkg/arch/`.

```bash
cd release/pkg/arch
makepkg -si
```

## Fedora / RHEL (RPM)

Use the included spec file template at `release/pkg/rpm/`.

```bash
rpmbuild -ba release/pkg/rpm/wayland-2-gnome.spec
```

## Debian / Ubuntu (DEB)

Use the included control file at `release/pkg/deb/`.

```bash
cd release/pkg/deb
dpkg-deb --build wayland-2-gnome
```

## NixOS

A Nix flake or derivation can be created from the standard `buildRustPackage` template.

---

# Verification

```bash
# Verify checksum
sha256sum -c wayland-2-gnome-0.1.0-linux-x86_64.tar.gz.sha256

# Verify binary works
./wayland-2-gnome --help

# Expected output:
# A protocol-aware Wayland proxy translating wlr-layer-shell to GNOME Shell overlays
# 
# Usage: wayland-2-gnome [OPTIONS]
# 
# Options:
#       --daemon       Run as a background daemon
#       --socket <SOCKET>  Bridge socket name [default: wayland-bridge-0]
#       --config <CONFIG>  Path to TOML config file
#   -h, --help         Print help
```
