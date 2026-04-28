#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

VERSION="$(grep '^version =' Cargo.toml | head -1 | cut -d'"' -f2)"
ARCH="$(uname -m)"
ARCHIVE="target/dist/wayland-2-gnome-${VERSION}-linux-${ARCH}.tar.gz"

if [ ! -f "$ARCHIVE" ]; then
  echo "Error: run 'cargo build --release && ./release/build.sh' first."
  echo "  Expected: $ARCHIVE"
  exit 1
fi

SIZE="$(du -h "$ARCHIVE" | cut -f1)"
CHECKSUM="$(cat "${ARCHIVE}.sha256" | cut -d' ' -f1)"

echo "=================================================="
echo " Release: v${VERSION}"
echo " Archive: ${ARCHIVE}"
echo " Size:    ${SIZE}"
echo " SHA256:  ${CHECKSUM}"
echo "=================================================="
echo ""
echo "Paste this as the GitHub release body:"
echo ""

cat << MARKDOWN
## v${VERSION}

$(grep "^## \\[${VERSION}\\]" CHANGELOG.md -A 9999 | sed -n '/^## \['"${VERSION}"'\]/,/^## \[/p' | sed '1d;$d')

### Assets

| File | SHA256 |
| :--- | :--- |
| \`wayland-2-gnome-${VERSION}-linux-${ARCH}.tar.gz\` | \`${CHECKSUM}\` |

### What's Inside

\`\`\`
wayland-2-gnome-${VERSION}-linux-${ARCH}/
  wayland-2-gnome           (static binary, ~2 MB)
  README.md
  LICENSE
  config.toml
  systemd/wayland-2-gnome.service
  autostart/wayland-2-gnome.desktop
\`\`\`

### Quick Install

\`\`\`bash
curl -LO https://github.com/leriart/Wayland-2-Gnome/releases/download/v${VERSION}/${ARCHIVE##*/}
tar xzf ${ARCHIVE##*/}
mkdir -p ~/.local/bin
cp wayland-2-gnome-${VERSION}-linux-${ARCH}/wayland-2-gnome ~/.local/bin/
wayland-2-gnome --help
\`\`\`
MARKDOWN
