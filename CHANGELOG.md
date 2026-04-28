# Changelog

All notable changes to Wayland 2 GNOME are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [0.1.2] - 2026-04-28

### Added

- **Phase 1 -- Dynamic multi-monitor**: TrackedOutput struct for wl_output geometry tracking. Post-collect global detection forwards new wl_output globals to clients dynamically. `sniff_output_event()` intercepts geometry changes for multi-monitor awareness.
- **Phase 2 -- Input re-injection**: MonitorInfo struct tracks display geometry and scale. `wl_pointer.motion` and `wl_pointer.enter` coordinates are offset by monitor origin bidirectionally, ensuring correct pointer positioning across multiple monitors.
- **Phase 3 -- Auto-install service**: `--install` flag writes `~/.config/systemd/user/wayland-2-gnome.service` with current `--socket` and `--compositor`, runs systemctl daemon-reload and enable, then exits.

## [0.1.1] — 2026-04-28

### Fixed
- Added `--compositor` CLI flag to allow specifying the compositor display name
  (previously hardcoded to `wayland-0`).

## [0.1.0] — 2026-04-28

### Added
- Sniff-and-forward proxy operating at the raw Wayland wire protocol level.
- `zwlr_layer_shell_v1` global injection when the compositor lacks it.
- Full layer surface translation: `get_layer_surface` -> `xdg_surface` + `xdg_toplevel` + `zxdg_decoration_manager_v1`.
- Interactive popup support with anchor and position translation.
- Output-aware multi-monitor targeting for layer surfaces.
- Orphan resource cleanup on client disconnect (destroy xdg_surfaces, toplevels, decorations).
- Daemon mode: `--daemon` flag, fork + setsid, PID file under `$XDG_RUNTIME_DIR`.
- TOML configuration file with hierarchical merge (defaults -> file -> CLI flags).
- calloop-based event loop replacing raw libc::poll for session dispatch.
- Graceful shutdown via LoopSignal (EOF on client/compositor disconnect).
- End-to-end integration tests with a mock compositor over raw Unix sockets.
- CLI flags: `--socket`, `--compositor`, `--max-clients`, `--log-level`, `--config`, `--pid-file`, `--daemon`.

### Changed
- Session loop migrated from `libc::poll` to `calloop::EventLoop` with `Generic<UnixStream>` sources.
- Accept loop uses nonblocking listener + sleep pattern, each session runs in its own thread.
- `SHUTDOWN_FLAG` moved to `proxy` module for test accessibility.
- Dual crate structure: `src/lib.rs` (library) and `src/main.rs` (binary) for testability.
- Build profile sets LTO = fat, codegen-units = 1, and symbol stripping for minimal binary size (~2 MB).

### Fixed
- Dead-code warnings eliminated (5 warnings to 0): `#[allow(dead_code)]` on `GlobalInfo.version`, `FakeObject`, `make_delete_id()`, `size()`.
- Collect phase fall-through bug: after injecting layer shell, the session loop no longer attempts a second blocking read from the compositor, preventing deadlock.
- Compilation fixes: `TranslationEntry` missing `#[derive(Clone)]`, missing `fn` on `handle_layer_surface_request`.

## [0.0.1] — 2025

### Added
- Initial proof-of-concept: raw byte-level proxy server.
- Phase 2: protocol-aware proxy using wayland_server crate.
- Phase 3: global injection and get_layer_surface interception.
- Phase 4: selective sniff-and-forward with OID rewriting.
- Full protocol translation for interactive popups, XDG aliasing, HiDPI handling.
- README and documentation.
