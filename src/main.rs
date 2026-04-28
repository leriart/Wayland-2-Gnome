//! Wayland GNOME Bridge
//!
//! Phase 1: Raw proxy server.
//! Listens on a Unix socket, accepts client connections,
//! connects to the real Wayland compositor (GNOME Mutter),
//! and proxies raw messages bidirectionally.

use std::os::unix::net::UnixListener;
use std::thread;

use anyhow::{Context, Result};
use log::{error, info};

mod proxy;

/// Bridge configuration
#[derive(Debug, Clone)]
struct BridgeConfig {
    /// Socket name (basename in $XDG_RUNTIME_DIR)
    socket_name: String,
    /// Real compositor's display (e.g., "wayland-0")
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

    let config = BridgeConfig::default();
    let socket_path = format!("{}/{}", socket_dir()?, config.socket_name);

    info!(
        "Starting bridge on '{}', proxying to '{}'",
        socket_path, config.compositor_display
    );

    // Remove old socket if present
    let _ = std::fs::remove_file(&socket_path);

    // Bind listener
    let listener =
        UnixListener::bind(&socket_path).context("Failed to bind socket")?;
    info!("Listening on {socket_path}");

    // Accept connections
    for stream in listener.incoming() {
        match stream {
            Ok(client) => {
                let display = config.compositor_display.clone();
                thread::spawn(move || {
                    if let Err(e) = proxy::handle_client(client, &display) {
                        error!("Client handler: {e}");
                    }
                });
            }
            Err(e) => error!("Accept failed: {e}"),
        }
    }

    Ok(())
}

fn socket_dir() -> Result<String> {
    let dir = std::env::var("XDG_RUNTIME_DIR")
        .context("XDG_RUNTIME_DIR is not set")?;
    Ok(dir)
}
