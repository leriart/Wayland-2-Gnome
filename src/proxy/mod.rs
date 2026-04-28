//! Phase 2: Protocol-aware proxy bridge.
//!
//! Uses wayland_server for the client-facing side (exposing globals,
//! dispatching requests) and will later add wayland_client for the
//! compositor-facing side (forwarding/translating to Mutter).

use std::sync::Arc;

use anyhow::{Context, Result};
use log::{error, info};
use wayland_server::protocol::{
    wl_compositor::WlCompositor,
    wl_data_device_manager::WlDataDeviceManager,
    wl_output::WlOutput,
    wl_seat::WlSeat,
    wl_shm::WlShm,
    wl_subcompositor::WlSubcompositor,
};
use wayland_server::{
    Client, DataInit, Dispatch, Display, DisplayHandle, GlobalDispatch,
    ListeningSocket,
};

use crate::BridgeConfig;

// ─── Bridge state ───────────────────────────────────────────────────────────

/// Shared state for the bridge server.
pub struct BridgeState;

// ─── Global dispatch implementations ────────────────────────────────────────
// Each one handles a specific global that our fake compositor advertises
// to connecting clients.

// -- wl_compositor (version 5) --
pub struct CompositorData;

impl GlobalDispatch<WlCompositor, CompositorData> for BridgeState {
    fn bind(
        _state: &mut Self,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: wayland_server::New<WlCompositor>,
        _data: &CompositorData,
        init: &mut DataInit<'_, Self>,
    ) {
        info!("Client bound wl_compositor");
        init.init(resource, ());
    }
}

impl Dispatch<WlCompositor, ()> for BridgeState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &WlCompositor,
        request: <WlCompositor as wayland_server::Resource>::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wayland_server::protocol::wl_compositor::Request::CreateSurface { id } => {
                info!("Client creates wl_surface (id={:?})", id);
            }
            wayland_server::protocol::wl_compositor::Request::CreateRegion { id } => {
                info!("Client creates wl_region (id={:?})", id);
            }
            _ => {}
        }
    }
}

// -- wl_subcompositor (version 1) --
pub struct SubcompositorData;

impl GlobalDispatch<WlSubcompositor, SubcompositorData> for BridgeState {
    fn bind(
        _state: &mut Self,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: wayland_server::New<WlSubcompositor>,
        _data: &SubcompositorData,
        init: &mut DataInit<'_, Self>,
    ) {
        info!("Client bound wl_subcompositor");
        init.init(resource, ());
    }
}

impl Dispatch<WlSubcompositor, ()> for BridgeState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &WlSubcompositor,
        _request: <WlSubcompositor as wayland_server::Resource>::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _init: &mut DataInit<'_, Self>,
    ) {
        info!("wl_subcompositor request");
    }
}

// -- wl_shm (version 1) --
pub struct ShmData;

impl GlobalDispatch<WlShm, ShmData> for BridgeState {
    fn bind(
        _state: &mut Self,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: wayland_server::New<WlShm>,
        _data: &ShmData,
        init: &mut DataInit<'_, Self>,
    ) {
        info!("Client bound wl_shm");
        init.init(resource, ());
    }
}

impl Dispatch<WlShm, ()> for BridgeState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &WlShm,
        _request: <WlShm as wayland_server::Resource>::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _init: &mut DataInit<'_, Self>,
    ) {
        info!("wl_shm request");
    }
}

// -- wl_data_device_manager (version 3) --
pub struct DataDeviceManagerData;

impl GlobalDispatch<WlDataDeviceManager, DataDeviceManagerData> for BridgeState {
    fn bind(
        _state: &mut Self,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: wayland_server::New<WlDataDeviceManager>,
        _data: &DataDeviceManagerData,
        init: &mut DataInit<'_, Self>,
    ) {
        info!("Client bound wl_data_device_manager");
        init.init(resource, ());
    }
}

impl Dispatch<WlDataDeviceManager, ()> for BridgeState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &WlDataDeviceManager,
        _request: <WlDataDeviceManager as wayland_server::Resource>::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _init: &mut DataInit<'_, Self>,
    ) {
        info!("wl_data_device_manager request");
    }
}

// -- wl_seat (version 8) --
pub struct SeatData;

impl GlobalDispatch<WlSeat, SeatData> for BridgeState {
    fn bind(
        _state: &mut Self,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: wayland_server::New<WlSeat>,
        _data: &SeatData,
        init: &mut DataInit<'_, Self>,
    ) {
        info!("Client bound wl_seat");
        init.init(resource, ());
    }
}

impl Dispatch<WlSeat, ()> for BridgeState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &WlSeat,
        _request: <WlSeat as wayland_server::Resource>::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _init: &mut DataInit<'_, Self>,
    ) {
        info!("wl_seat request");
    }
}

// -- wl_output (version 4) --
pub struct OutputData;

impl GlobalDispatch<WlOutput, OutputData> for BridgeState {
    fn bind(
        _state: &mut Self,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: wayland_server::New<WlOutput>,
        _data: &OutputData,
        init: &mut DataInit<'_, Self>,
    ) {
        info!("Client bound wl_output");
        init.init(resource, ());
    }
}

impl Dispatch<WlOutput, ()> for BridgeState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &WlOutput,
        _request: <WlOutput as wayland_server::Resource>::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _init: &mut DataInit<'_, Self>,
    ) {
        info!("wl_output request");
    }
}

// ─── Event loop ─────────────────────────────────────────────────────────────

/// Run the main event loop.
pub fn run_event_loop(
    mut display: Display<BridgeState>,
    socket: ListeningSocket,
    _config: Arc<BridgeConfig>,
) -> Result<()> {
    let mut state = BridgeState;

    // Register globals the bridge exposes to clients
    {
        let dh = display.handle();
        dh.create_global::<BridgeState, WlCompositor, CompositorData>(
            5,
            CompositorData,
        );
        dh.create_global::<BridgeState, WlSubcompositor, SubcompositorData>(
            1,
            SubcompositorData,
        );
        dh.create_global::<BridgeState, WlShm, ShmData>(1, ShmData);
        dh.create_global::<BridgeState, WlDataDeviceManager, DataDeviceManagerData>(
            3,
            DataDeviceManagerData,
        );
        dh.create_global::<BridgeState, WlSeat, SeatData>(8, SeatData);
        dh.create_global::<BridgeState, WlOutput, OutputData>(4, OutputData);
        info!("Registered 6 globals");
    }

    info!("Event loop running. Press Ctrl+C to stop.");

    // Main loop: poll for new connections + dispatch client events
    //
    // In a production version, this would use calloop or epoll to
    // multiplex the ListeningSocket fd and the display fd together.
    // For Phase 2, a simple poll loop suffices.
    loop {
        // 1. Accept any new client connections
        match socket.accept() {
            Ok(Some(stream)) => {
                info!("New client connection accepted");
                let mut dh = display.handle();
                // Insert the client into the Wayland display
                //
                // Note: wayland_server 0.31 requires inserting the client
                // via DisplayHandle. The backend handles dispatch.
                // We pass a minimal ClientData implementation to track the client.
                use wayland_backend::server::ClientData;
                use std::sync::Arc;

                let client_data = Arc::new(BridgeClientData);
                match dh.insert_client(stream, client_data as Arc<dyn ClientData>) {
                    Ok(_client) => {
                        info!("Client registered in display");
                    }
                    Err(e) => {
                        error!("Failed to insert client: {e}");
                    }
                }
            }
            Ok(None) => {
                // No pending connections — normal
            }
            Err(e) => {
                error!("Accept error: {e}");
            }
        }

        // 2. Dispatch pending Wayland events from all clients
        if let Err(e) = display.dispatch_clients(&mut state) {
            error!("Dispatch error: {e}");
            break;
        }

        // 3. Flush outgoing buffers
        if let Err(e) = display.flush_clients() {
            error!("Flush error: {e}");
            break;
        }

        // 4. Prevent busy-loop
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    info!("Event loop exited");
    Ok(())
}

/// Minimal ClientData implementation for tracking connected clients.
struct BridgeClientData;

impl wayland_backend::server::ClientData for BridgeClientData {
    fn initialized(&self, _client_id: wayland_backend::server::ClientId) {
        info!("Client initialized");
    }

    fn disconnected(&self, _client_id: wayland_backend::server::ClientId, _reason: wayland_backend::server::DisconnectReason) {
        info!("Client disconnected");
    }
}
