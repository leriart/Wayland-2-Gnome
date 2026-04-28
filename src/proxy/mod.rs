//! Core proxy loop: transparently forwards Wayland protocol messages
//! between a client and the real compositor (GNOME Mutter).
//!
//! Phase 1: Byte-level passthrough (no protocol awareness yet).
//! Phase 2+: Protocol interception (filter/modify globals, translate
//!           wlr-layer-shell to Mutter-compatible calls).

use std::io::{self, Read, Write};
use std::os::unix::io::{AsRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::thread;

use anyhow::{bail, Context, Result};
use log::{error, info, warn};
use wayland_backend::server::Backend;
use wayland_server::Display;

use crate::BridgeConfig;

/// Socket path for the bridge
fn bridge_socket_path(socket_name: &str) -> Result<String> {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
        .context("XDG_RUNTIME_DIR not set")?;
    Ok(format!("{}/{}", runtime_dir, socket_name))
}

/// Phase 1 entry point: create the server socket and accept connections.
///
/// Each client connection gets its own thread with a raw bidirectional proxy.
pub fn run_proxy(config: &BridgeConfig) -> Result<()> {
    let socket_path = bridge_socket_path(&config.socket_name)?;

    // Remove old socket if it exists (from a previous crashed instance)
    let _ = std::fs::remove_file(&socket_path);

    // Create a wayland server display
    let mut display = Display::new();
    display
        .add_socket_named(&config.socket_name)
        .context("Failed to create Wayland server socket")?;

    info!("Listening on {}", socket_path);

    // Connect to the real compositor
    let compositor_display = config.compositor_display.clone();

    // The wayland_server Display handles accepting connections automatically.
    // For the raw passthrough, we need to:
    // 1. Accept client connections (wayland_server handles this)
    // 2. Connect to real compositor (wayland_client)
    // 3. Forward raw bytes bidirectionally
    //
    // Since we need raw fd access for custom passthrough, we'll use
    // UnixStream directly for Phase 1 rather than higher-level wayland_server APIs.

    info!(
        "Proxy listening. Set WAYLAND_DISPLAY={} and run your app.",
        config.socket_name
    );

    // For Phase 1, we'll implement a simple raw proxy using Unix sockets.
    // This bypasses wayland_server's abstractions to give us full control
    // over the byte stream — necessary for transparent passthrough.

    // Listen for client connections
    let listener = std::os::unix::net::UnixListener::bind(&socket_path)
        .context("Failed to bind to socket")?;

    info!("Raw listener bound to {}", socket_path);

    // Accept connections in a loop
    for client_stream in listener.incoming() {
        match client_stream {
            Ok(client) => {
                let compositor_display = compositor_display.clone();
                thread::spawn(move || {
                    if let Err(e) = handle_client(client, &compositor_display) {
                        error!("Client handler error: {}", e);
                    }
                });
            }
            Err(e) => {
                error!("Failed to accept client: {}", e);
            }
        }
    }

    Ok(())
}

/// Handle a single client connection: connect to compositor and proxy bytes.
fn handle_client(
    mut client: UnixStream,
    compositor_display: &str,
) -> Result<()> {
    info!("New client connection, connecting to '{}'", compositor_display);
    
    // Connect to the real compositor
    let compositor_path = bridge_socket_path(compositor_display)?;
    let mut compositor = UnixStream::connect(&compositor_path)
        .context("Failed to connect to compositor")?;

    info!("Bidirectional proxy established");

    // Raw byte passthrough: two-way copy between client and compositor.
    // Phase 2+ will replace this with protocol-aware dispatching.
    //
    // For now, this proves the proxying mechanism works.
    // It won't work for real apps yet because Wayland also needs to
    // transfer file descriptors (dmabufs, etc.) which raw UnixStream
    // doesn't handle via read/write — we need sendmsg/recvmsg with
    // SCM_RIGHTS for proper fd passthrough.

    // Phase 1 limitation: this works for protocol negotiation but won't
    // handle buffer sharing. That's intentional — Phase 1 proves the
    // connection model, Phase 2 adds protocol awareness.

    let mut client_clone = client.try_clone()?;
    let mut compositor_clone = compositor.try_clone()?;

    let handle = thread::spawn(move || {
        io::copy(&mut compositor_clone, &mut client)
            .map_err(|e| warn!("Compositor→Client copy ended: {}", e))
    });

    io::copy(&mut client_clone, &mut compositor)
        .map_err(|e| warn!("Client→Compositor copy ended: {}", e))?;

    let _ = handle.join();

    info!("Client disconnected");
    Ok(())
}
