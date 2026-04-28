# Release Checklist

Use this checklist when cutting a new release. Run steps in order.

## 1. Update Version

```bash
# Edit Cargo.toml
sed -i 's/^version = "0.1.0"/version = "0.1.1"/' Cargo.toml
```

Follow semver:
- **Patch** (0.1.0 → 0.1.1): bug fixes, minor improvements.
- **Minor** (0.1.0 → 0.2.0): new features, protocol changes.
- **Major** (0.1.0 → 1.0.0): stable API, breaking changes.

## 2. Update CHANGELOG.md

Add a new section at the top following Keep a Changelog format.

## 3. Run Tests

```bash
cargo test
cargo check
cargo build --release
```

## 4. Build Release Archive

```bash
./release/build.sh
```

Produces:
- `target/dist/wayland-2-gnome-0.1.1-linux-x86_64.tar.gz`
- `target/dist/wayland-2-gnome-0.1.1-linux-x86_64.tar.gz.sha256`

## 5. Generate GitHub Release Body

```bash
./release/draft-gh-release.sh
```

## 6. Publish on GitHub

1. https://github.com/leriart/Wayland-2-Gnome/releases → "Create a new release"
2. Tag: `v0.1.1`
3. Title: `v0.1.1`
4. Paste body from `draft-gh-release.sh` output
5. Attach the `.tar.gz` and `.sha256` files
6. Publish

# Installation Methods

## Manual (all distros)

```bash
curl -LO https://github.com/leriart/Wayland-2-Gnome/releases/download/v0.1.1/wayland-2-gnome-0.1.1-linux-x86_64.tar.gz
tar xzf wayland-2-gnome-0.1.1-linux-x86_64.tar.gz
cp wayland-2-gnome-0.1.1-linux-x86_64/wayland-2-gnome ~/.local/bin/
mkdir -p ~/.config/autostart
cp wayland-2-gnome-0.1.1-linux-x86_64/autostart/wayland-2-gnome.desktop ~/.config/autostart/
```

## Systemd (user service)

```bash
cp wayland-2-gnome-0.1.1-linux-x86_64/wayland-2-gnome ~/.local/bin/
mkdir -p ~/.config/systemd/user
cp wayland-2-gnome-0.1.1-linux-x86_64/systemd/wayland-2-gnome.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now wayland-2-gnome
```

## Build from Source

```bash
git clone https://github.com/leriart/Wayland-2-Gnome
cd Wayland-2-Gnome
cargo build --release
cp target/release/wayland-2-gnome ~/.local/bin/
```
