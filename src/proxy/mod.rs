//! Phase 1 raw proxy: connects to the real compositor and copies bytes
//! bidirectionally. This is a minimal proof that proxying works.
//!
//! Note: Wayland uses file descriptor passing (SCM_RIGHTS) for shared
//! memory and dmabufs. Raw io::copy doesn't handle that — so Phase 1
//! is a connection test, not a fully working proxy. Phase 2 adds
//! proper protocol-aware dispatching with fd forwarding.

use std::io;
use std::os::unix::net::UnixStream;
use std::thread;

use anyhow::{Context, Result};
use log::info;

/// Create a socket path from display name.
pub fn compositor_socket_path(display: &str) -> Result<String> {
    let dir = std::env::var("XDG_RUNTIME_DIR")
        .context("XDG_RUNTIME_DIR not set")?;
    Ok(format!("{dir}/{display}"))
}

/// Handle one client connection: bridge to the real compositor.
pub fn handle_client(mut client: UnixStream, display: &str) -> Result<()> {
    let compositor_path = compositor_socket_path(display)?;
    info!("Client connected → connecting to compositor at {compositor_path}");

    let mut compositor =
        UnixStream::connect(&compositor_path).context("connect compositor")?;

    info!("Bidirectional proxy established");

    // Clone for the two directions
    let mut client_reader = client.try_clone()?;
    let mut compositor_reader = compositor.try_clone()?;

    // Spawn thread for compositor → client direction
    let h = thread::spawn(move || {
        let _ = io::copy(&mut compositor_reader, &mut client);
    });

    // Main thread: client → compositor
    let _ = io::copy(&mut client_reader, &mut compositor);

    // Wait for the other direction
    let _ = h.join();

    info!("Client disconnected");
    Ok(())
}
