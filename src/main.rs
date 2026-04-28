//! Wayland GNOME Bridge - Phase 1: Raw passthrough proxy
//!
//! This is the MVP: a transparent byte-level proxy that sits between
//! a Wayland client and GNOME Mutter. No protocol awareness yet.
//!
//! Usage:
//!   WAYLAND_DISPLAY=wayland-gnome-bridge-0 cava-bg on
//!   (or set the env var automatically via wrapper script)

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process;
use std::sync::Arc;

use anyhow::{Context, Result};
use log::{error, info, warn};
use nix::sys::stat::Mode;
use nix::unistd::{mkfifo, unlink};
use wayland_backend::server::Backend;
use wayland_server::Display;

mod proxy;

/// Default socket name for the bridge
const SOCKET_NAME: &str = "wayland-gnome-bridge-0";

/// Bridge configuration
#[derive(Debug, Clone)]
struct BridgeConfig {
    /// Socket name (basename in $XDG_RUNTIME_DIR)
    socket_name: String,
    /// Real compositor's display (e.g., "wayland-0")
    compositor_display: String,
    /// Verbose logging
    debug: bool,
    /// PID file path
    pid_file: Option<PathBuf>,
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            socket_name: SOCKET_NAME.to_string(),
            compositor_display: env::var("WAYLAND_DISPLAY")
                .unwrap_or_else(|_| "wayland-0".to_string()),
            debug: false,
            pid_file: None,
        }
    }
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info"),
    )
    .init();

    let config = BridgeConfig {
        ..Default::default()
    };

    info!(
        "Starting Wayland GNOME Bridge on socket '{}', proxying to '{}'",
        config.socket_name, config.compositor_display
    );

    // Phase 1: Just set up the server side and connect to real compositor
    // For now, this is a stub that proves the project compiles and runs.
    //
    // The real proxy loop will:
    // 1. Create a wayland_server::Display
    // 2. Listen on a socket in $XDG_RUNTIME_DIR
    // 3. Accept client connections
    // 4. For each client: fork a proxy handler that:
    //    a. Connects to the real compositor (wayland-client)
    //    b. Forwards all messages bidirectionally
    // 5. Run a calloop event loop

    proxy::run_proxy(&config).context("Failed to run proxy")?;

    Ok(())
}
