//! Phase 3a: Layer-shell proxy bridge.
//!
//! Exposes `zwlr_layer_shell_v1` global, intercepts `get_layer_surface`,
//! and manages layer surface lifecycle — still without compositor backend.

use std::sync::Arc;

use anyhow::Result;
use log::{error, info, warn};
use wayland_backend::server::{ClientData, ClientId, DisconnectReason};
use wayland_protocols_wlr::layer_shell::v1::server::{
    zwlr_layer_shell_v1::ZwlrLayerShellV1,
    zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
};
use wayland_server::protocol::{
    wl_compositor::WlCompositor,
    wl_data_device_manager::WlDataDeviceManager,
    wl_output::WlOutput,
    wl_region::WlRegion,
    wl_seat::WlSeat,
    wl_shm::WlShm,
    wl_subcompositor::WlSubcompositor,
    wl_surface::WlSurface,
};
use wayland_server::{
    Client, DataInit, Dispatch, Display, DisplayHandle, GlobalDispatch,
    ListeningSocket, Resource,
};

use crate::BridgeConfig;

// ─── Types for global user data ─────────────────────────────────────────────

pub struct CompositorGlobal;
pub struct SubcompositorGlobal;
pub struct ShmGlobal;
pub struct DataDeviceManagerGlobal;
pub struct SeatGlobal;
pub struct OutputGlobal;
pub struct LayerShellGlobal;

// ─── Types for resource user data ───────────────────────────────────────────

/// Data attached to each `zwlr_layer_surface_v1` instance.
pub struct LayerSurfaceData {
    /// The parent `wl_surface` this layer surface was created on.
    pub surface_id: wayland_backend::server::ObjectId,
    /// The namespace provided by the client.
    pub namespace: String,
    /// Which layer (background, bottom, top, overlay).
    pub layer: u32,
    /// Client ID for tracking.
    pub client_id: ClientId,
}

// ─── Bridge state ───────────────────────────────────────────────────────────

/// Shared state for the bridge server.
pub struct BridgeState {
    /// Number of layer surfaces created (for debugging).
    pub layer_surface_count: u64,
}

// ─── GlobalDispatch implementations ─────────────────────────────────────────

impl GlobalDispatch<WlCompositor, CompositorGlobal> for BridgeState {
    fn bind(
        _state: &mut Self,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: wayland_server::New<WlCompositor>,
        _data: &CompositorGlobal,
        init: &mut DataInit<'_, Self>,
    ) {
        info!("Client bound wl_compositor");
        init.init(resource, ());
    }
}

impl GlobalDispatch<WlSubcompositor, SubcompositorGlobal> for BridgeState {
    fn bind(
        _state: &mut Self,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: wayland_server::New<WlSubcompositor>,
        _data: &SubcompositorGlobal,
        init: &mut DataInit<'_, Self>,
    ) {
        info!("Client bound wl_subcompositor");
        init.init(resource, ());
    }
}

impl GlobalDispatch<WlShm, ShmGlobal> for BridgeState {
    fn bind(
        _state: &mut Self,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: wayland_server::New<WlShm>,
        _data: &ShmGlobal,
        init: &mut DataInit<'_, Self>,
    ) {
        info!("Client bound wl_shm");
        init.init(resource, ());
    }
}

impl GlobalDispatch<WlDataDeviceManager, DataDeviceManagerGlobal> for BridgeState {
    fn bind(
        _state: &mut Self,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: wayland_server::New<WlDataDeviceManager>,
        _data: &DataDeviceManagerGlobal,
        init: &mut DataInit<'_, Self>,
    ) {
        info!("Client bound wl_data_device_manager");
        init.init(resource, ());
    }
}

impl GlobalDispatch<WlSeat, SeatGlobal> for BridgeState {
    fn bind(
        _state: &mut Self,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: wayland_server::New<WlSeat>,
        _data: &SeatGlobal,
        init: &mut DataInit<'_, Self>,
    ) {
        info!("Client bound wl_seat");
        init.init(resource, ());
    }
}

impl GlobalDispatch<WlOutput, OutputGlobal> for BridgeState {
    fn bind(
        _state: &mut Self,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: wayland_server::New<WlOutput>,
        _data: &OutputGlobal,
        init: &mut DataInit<'_, Self>,
    ) {
        info!("Client bound wl_output");
        init.init(resource, ());
    }
}

/// ⭐ The main attraction: `zwlr_layer_shell_v1` global dispatch.
impl GlobalDispatch<ZwlrLayerShellV1, LayerShellGlobal> for BridgeState {
    fn bind(
        _state: &mut Self,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: wayland_server::New<ZwlrLayerShellV1>,
        _data: &LayerShellGlobal,
        init: &mut DataInit<'_, Self>,
    ) {
        info!("Client bound zwlr_layer_shell_v1");
        init.init(resource, LayerShellGlobal);
    }
}

// ─── Dispatch for core protocol resources ───────────────────────────────────

impl Dispatch<WlCompositor, ()> for BridgeState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &WlCompositor,
        request: <WlCompositor as Resource>::Request,
        _data: &(),
        _dh: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wayland_server::protocol::wl_compositor::Request::CreateSurface { id } => {
                info!("Client creates wl_surface");
                init.init(id, ());
            }
            wayland_server::protocol::wl_compositor::Request::CreateRegion { id } => {
                info!("Client creates wl_region");
                init.init(id, ());
            }
            _ => {}
        }
    }
}

impl Dispatch<WlRegion, ()> for BridgeState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &WlRegion,
        _request: <WlRegion as Resource>::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _init: &mut DataInit<'_, Self>,
    ) {
        // wl_region requests — accept silently
    }
}

impl Dispatch<WlSubcompositor, ()> for BridgeState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &WlSubcompositor,
        _request: <WlSubcompositor as Resource>::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _init: &mut DataInit<'_, Self>,
    ) {
        info!("wl_subcompositor request");
    }
}

impl Dispatch<WlShm, ()> for BridgeState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &WlShm,
        _request: <WlShm as Resource>::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _init: &mut DataInit<'_, Self>,
    ) {
        // wl_shm requests — accept silently
    }
}

impl Dispatch<WlDataDeviceManager, ()> for BridgeState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &WlDataDeviceManager,
        _request: <WlDataDeviceManager as Resource>::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _init: &mut DataInit<'_, Self>,
    ) {
        info!("wl_data_device_manager request");
    }
}

impl Dispatch<WlSeat, ()> for BridgeState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &WlSeat,
        _request: <WlSeat as Resource>::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _init: &mut DataInit<'_, Self>,
    ) {
        info!("wl_seat request");
    }
}

impl Dispatch<WlOutput, ()> for BridgeState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &WlOutput,
        _request: <WlOutput as Resource>::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _init: &mut DataInit<'_, Self>,
    ) {
        // wl_output requests
    }
}

// ─── Dispatch for WlSurface ─────────────────────────────────────────────────

impl Dispatch<WlSurface, ()> for BridgeState {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _resource: &WlSurface,
        request: <WlSurface as Resource>::Request,
        _data: &(),
        _dh: &DisplayHandle,
        _init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wayland_server::protocol::wl_surface::Request::Attach {
                buffer, x, y, ..
            } => {
                let has_buffer = buffer.is_some();
                info!(
                    "wl_surface.attach(buffer={}, x={}, y={})",
                    has_buffer, x, y
                );
            }
            wayland_server::protocol::wl_surface::Request::Commit { .. } => {}
            wayland_server::protocol::wl_surface::Request::Damage {
                x, y, width, height,
                ..
            } => {
                info!("wl_surface.damage({},{},{},{})", x, y, width, height);
            }
            _ => {}
        }
    }
}

// ─── ⭐ Dispatch for zwlr_layer_shell_v1 (server side) ──────────────────────

impl Dispatch<ZwlrLayerShellV1, LayerShellGlobal> for BridgeState {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &ZwlrLayerShellV1,
        request: <ZwlrLayerShellV1 as Resource>::Request,
        _data: &LayerShellGlobal,
        dh: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wayland_protocols_wlr::layer_shell::v1::server::zwlr_layer_shell_v1::Request::GetLayerSurface {
                id,
                surface,
                output,
                layer,
                namespace,
            } => {
                state.layer_surface_count += 1;
                let n = state.layer_surface_count;

                // Convert WEnum to u32 for logging
                let layer_val: u32 = layer.into();

                info!(
                    "⭐ get_layer_surface #{}: layer={}, namespace='{}', output={:?}",
                    n, layer_val, namespace, output
                );

                // Create the layer surface resource data
                let surface_id = surface.id();
                let surface_data = LayerSurfaceData {
                    surface_id,
                    namespace: namespace.to_string(),
                    layer: layer_val,
                    client_id: _client.id(),
                };

                // Initialize the resource (returns the initialized resource)
                let layer_surface = init.init(id, surface_data);

                // Send the configure event — required by protocol before client
                // can attach a buffer. Serial 1 for the initial configure.
                // We send width=0, height=0 to let the client decide its own size.
                layer_surface.configure(0, 0, 1);
                info!("  ⭐ zwlr_layer_surface_v1 configured (serial=1)");
            }
            wayland_protocols_wlr::layer_shell::v1::server::zwlr_layer_shell_v1::Request::Destroy { .. } => {
                info!("zwlr_layer_shell_v1.destroy");
            }
            _ => {
                warn!("Unhandled zwlr_layer_shell_v1 request");
            }
        }
    }
}

// ─── Dispatch for zwlr_layer_surface_v1 ─────────────────────────────────────

impl Dispatch<ZwlrLayerSurfaceV1, LayerSurfaceData> for BridgeState {
    fn request(
        state: &mut Self,
        _client: &Client,
        _resource: &ZwlrLayerSurfaceV1,
        request: <ZwlrLayerSurfaceV1 as Resource>::Request,
        _data: &LayerSurfaceData,
        _dh: &DisplayHandle,
        _init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wayland_protocols_wlr::layer_shell::v1::server::zwlr_layer_surface_v1::Request::SetSize { width, height } => {
                info!(
                    "  layer_surface #{} set_size({}x{})",
                    state.layer_surface_count, width, height
                );
            }
            wayland_protocols_wlr::layer_shell::v1::server::zwlr_layer_surface_v1::Request::SetAnchor { anchor } => {
                let anchor_val: u32 = anchor.into();
                info!(
                    "  layer_surface #{} set_anchor(0x{:x})",
                    state.layer_surface_count, anchor_val
                );
            }
            wayland_protocols_wlr::layer_shell::v1::server::zwlr_layer_surface_v1::Request::SetExclusiveZone { zone } => {
                info!(
                    "  layer_surface #{} set_exclusive_zone({})",
                    state.layer_surface_count, zone
                );
            }
            wayland_protocols_wlr::layer_shell::v1::server::zwlr_layer_surface_v1::Request::SetMargin {
                top,
                right,
                bottom,
                left,
            } => {
                info!(
                    "  layer_surface #{} set_margin(t={},r={},b={},l={})",
                    state.layer_surface_count, top, right, bottom, left
                );
            }
            wayland_protocols_wlr::layer_shell::v1::server::zwlr_layer_surface_v1::Request::SetKeyboardInteractivity {
                keyboard_interactivity,
            } => {
                let ki_val: u32 = keyboard_interactivity.into();
                info!(
                    "  layer_surface #{} set_keyboard_interactivity({})",
                    state.layer_surface_count, ki_val
                );
            }
            wayland_protocols_wlr::layer_shell::v1::server::zwlr_layer_surface_v1::Request::GetPopup { popup, .. } => {
                info!(
                    "  layer_surface #{} get_popup (xdg_popup not yet implemented, popup={:?})",
                    state.layer_surface_count, popup
                );
                // Popup is already created by the client; we just acknowledge it.
            }
            wayland_protocols_wlr::layer_shell::v1::server::zwlr_layer_surface_v1::Request::AckConfigure { serial } => {
                info!(
                    "  layer_surface #{} ack_configure({})",
                    state.layer_surface_count, serial
                );
            }
            wayland_protocols_wlr::layer_shell::v1::server::zwlr_layer_surface_v1::Request::Destroy { .. } => {
                info!(
                    "  layer_surface #{} destroy",
                    state.layer_surface_count
                );
            }
            _ => {
                warn!("Unhandled zwlr_layer_surface_v1 request");
            }
        }
    }
}

// ─── Event loop ─────────────────────────────────────────────────────────────

/// Run the main event loop.
pub fn run_event_loop(
    mut display: Display<BridgeState>,
    socket: ListeningSocket,
    _config: Arc<BridgeConfig>,
) -> Result<()> {
    let mut state = BridgeState {
        layer_surface_count: 0,
    };

    // Register globals
    {
        let dh = display.handle();
        dh.create_global::<BridgeState, WlCompositor, CompositorGlobal>(
            5,
            CompositorGlobal,
        );
        dh.create_global::<BridgeState, WlSubcompositor, SubcompositorGlobal>(
            1,
            SubcompositorGlobal,
        );
        dh.create_global::<BridgeState, WlShm, ShmGlobal>(1, ShmGlobal);
        dh.create_global::<BridgeState, WlDataDeviceManager, DataDeviceManagerGlobal>(
            3,
            DataDeviceManagerGlobal,
        );
        dh.create_global::<BridgeState, WlSeat, SeatGlobal>(8, SeatGlobal);
        dh.create_global::<BridgeState, WlOutput, OutputGlobal>(4, OutputGlobal);

        // ⭐ The layer shell global — version 4
        dh.create_global::<BridgeState, ZwlrLayerShellV1, LayerShellGlobal>(
            4,
            LayerShellGlobal,
        );

        info!("Registered 7 globals (including zwlr_layer_shell_v1)");
    }

    info!("⏳ Bridge Phase 3a running. Waiting for clients...");

    loop {
        // 1. Accept new clients
        match socket.accept() {
            Ok(Some(stream)) => {
                info!("New client connection accepted");
                let client_data = Arc::new(BridgeClientData);
                let mut dh = display.handle();
                if let Err(e) = dh.insert_client(stream, client_data as Arc<dyn ClientData>) {
                    error!("Failed to insert client: {e}");
                }
            }
            Ok(None) => {}
            Err(e) => {
                error!("Accept error: {e}");
            }
        }

        // 2. Dispatch events
        if let Err(e) = display.dispatch_clients(&mut state) {
            error!("Dispatch error: {e}");
            break;
        }

        // 3. Flush
        if let Err(e) = display.flush_clients() {
            error!("Flush error: {e}");
            break;
        }

        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    info!("Event loop exited");
    Ok(())
}

// ─── Client tracking ────────────────────────────────────────────────────────

struct BridgeClientData;

impl ClientData for BridgeClientData {
    fn initialized(&self, _client_id: ClientId) {
        info!("Client initialized (id={:?})", _client_id);
    }

    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {
        info!("Client disconnected (id={:?})", _client_id);
    }
}
