//! Wayland GNOME Bridge — Phase 2
//!
//! Protocol-aware proxy using wayland_server + wayland_client.
//! Sits between apps and GNOME Mutter, translating wlr-layer-shell
//! into operations Mutter understands.

use std::fs;
use std::sync::Arc;

use anyhow::{Context, Result};
use log::{error, info};
use wayland_server::{Display, ListeningSocket};

mod proxy;

/// Bridge configuration
#[derive(Debug, Clone)]
struct BridgeConfig {
    socket_name: String,
    compositor_display: String,
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            socket_name: "wayland-bridge-0".to_string(),
            compositor_display: std::env::var("WAYLAND_DISPLAY")
                .unwrap_or_else(|_| "wayland-0".to_string()),
        }
    }
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info"),
    )
    .init();

    let config = Arc::new(BridgeConfig::default());
    let socket_path = format!("{}/{}", socket_dir()?, config.socket_name);

    info!(
        "Starting bridge Phase 2 on '{}', proxying to '{}'",
        socket_path, config.compositor_display
    );

    // Remove old socket if present
    let _ = fs::remove_file(&socket_path);
    let _ = fs::remove_file(format!("{}.lock", &socket_path));

    // Create the Wayland display (server side)
    let display: Display<proxy::BridgeState> = Display::new()
        .context("Failed to create Wayland display")?;

    // Bind the Unix socket using ListeningSocket utility
    let socket = ListeningSocket::bind(&config.socket_name)
        .context("Failed to bind socket")?;

    info!(
        "Server socket ready at {}",
        socket_path
    );

    // Run the event loop — passes ownership of display and socket
    proxy::run_event_loop(display, socket, config)?;

    Ok(())
}

fn socket_dir() -> Result<String> {
    Ok(std::env::var("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR not set")?)
}
