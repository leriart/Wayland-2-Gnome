//! Phase 3b: Proxy bridge with compositor backend.
//!
//! Exposes `zwlr_layer_shell_v1` global and translates intercepted
//! requests to real Mutter (wayland-1) calls via `wayland_client`.

use std::sync::Arc;

use anyhow::{Context, Result};
use log::{error, info, warn};
use wayland_backend::server::{ClientData, ClientId, DisconnectReason};
use wayland_client::backend::{Backend, ObjectData};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_compositor::WlCompositor as ClientWlCompositor;
use wayland_client::protocol::wl_output::WlOutput as ClientWlOutput;
use wayland_client::protocol::wl_registry::WlRegistry as ClientWlRegistry;
use wayland_client::protocol::wl_seat::WlSeat as ClientWlSeat;
use wayland_client::protocol::wl_surface::WlSurface as ClientWlSurface;
use wayland_client::{Connection, EventQueue, QueueHandle};
use wayland_protocols_wlr::layer_shell::v1::server::{
    zwlr_layer_shell_v1 as server_shell,
    zwlr_layer_surface_v1 as server_surface,
};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1 as client_shell,
    zwlr_layer_surface_v1 as client_surface,
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
    Client, DataInit, Dispatch, Display, DisplayHandle,
    GlobalDispatch, ListeningSocket, Resource,
};

use crate::BridgeConfig;

pub struct CompositorGlobal;
pub struct SubcompositorGlobal;
pub struct ShmGlobal;
pub struct DataDeviceManagerGlobal;
pub struct SeatGlobal;
pub struct OutputGlobal;
pub struct LayerShellGlobal;

// ─── Client-side ObjectData ─────────────────────────────────────────────────

struct ClientCompositorData;
impl ObjectData for ClientCompositorData {
    fn event(
        self: Arc<Self>,
        _backend: &Backend,
        _msg: wayland_client::backend::protocol::Message<
            wayland_backend::client::ObjectId,
            std::os::unix::io::OwnedFd,
        >,
    ) -> Option<Arc<dyn ObjectData>> {
        None
    }
    fn destroyed(&self, _object_id: wayland_backend::client::ObjectId) {}
    fn debug(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ClientCompositorData")
    }
}

struct ClientLayerShellData;
impl ObjectData for ClientLayerShellData {
    fn event(
        self: Arc<Self>,
        _backend: &Backend,
        _msg: wayland_client::backend::protocol::Message<
            wayland_backend::client::ObjectId,
            std::os::unix::io::OwnedFd,
        >,
    ) -> Option<Arc<dyn ObjectData>> {
        None
    }
    fn destroyed(&self, _object_id: wayland_backend::client::ObjectId) {}
    fn debug(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ClientLayerShellData")
    }
}

struct ClientOutputData;
impl ObjectData for ClientOutputData {
    fn event(
        self: Arc<Self>,
        _backend: &Backend,
        _msg: wayland_client::backend::protocol::Message<
            wayland_backend::client::ObjectId,
            std::os::unix::io::OwnedFd,
        >,
    ) -> Option<Arc<dyn ObjectData>> {
        None
    }
    fn destroyed(&self, _object_id: wayland_backend::client::ObjectId) {}
    fn debug(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ClientOutputData")
    }
}

struct ClientSeatData;
impl ObjectData for ClientSeatData {
    fn event(
        self: Arc<Self>,
        _backend: &Backend,
        _msg: wayland_client::backend::protocol::Message<
            wayland_backend::client::ObjectId,
            std::os::unix::io::OwnedFd,
        >,
    ) -> Option<Arc<dyn ObjectData>> {
        None
    }
    fn destroyed(&self, _object_id: wayland_backend::client::ObjectId) {}
    fn debug(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ClientSeatData")
    }
}

struct ClientSurfaceData;
impl ObjectData for ClientSurfaceData {
    fn event(
        self: Arc<Self>,
        _backend: &Backend,
        _msg: wayland_client::backend::protocol::Message<
            wayland_backend::client::ObjectId,
            std::os::unix::io::OwnedFd,
        >,
    ) -> Option<Arc<dyn ObjectData>> {
        None
    }
    fn destroyed(&self, _object_id: wayland_backend::client::ObjectId) {}
    fn debug(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ClientSurfaceData")
    }
}

struct ClientLayerSurfaceData;
impl ObjectData for ClientLayerSurfaceData {
    fn event(
        self: Arc<Self>,
        _backend: &Backend,
        msg: wayland_client::backend::protocol::Message<
            wayland_backend::client::ObjectId,
            std::os::unix::io::OwnedFd,
        >,
    ) -> Option<Arc<dyn ObjectData>> {
        info!("  [Mutter→client] layer_surface event opcode={}", msg.opcode);
        None
    }
    fn destroyed(&self, _object_id: wayland_backend::client::ObjectId) {}
    fn debug(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ClientLayerSurfaceData")
    }
}

// ─── Layer surface pairing ──────────────────────────────────────────────────

pub struct LayerSurfaceData {
    pub namespace: String,
    pub layer: u32,
    pub client_id: ClientId,
    pub client_layer_surface: Option<client_surface::ZwlrLayerSurfaceV1>,
}

// ─── Bridge state ───────────────────────────────────────────────────────────

pub struct BridgeState {
    pub layer_surface_count: u64,
    pub client_conn: Connection,
    pub client_event_queue: EventQueue<BridgeState>,
    pub client_compositor: Option<ClientWlCompositor>,
    pub client_layer_shell: Option<client_shell::ZwlrLayerShellV1>,
    pub client_outputs: Vec<ClientWlOutput>,
    pub client_seat: Option<ClientWlSeat>,
    pub client_registry: Option<ClientWlRegistry>,
}

impl ::wayland_client::Dispatch<ClientWlRegistry, GlobalListContents> for BridgeState {
    fn event(
        _state: &mut Self,
        _registry: &ClientWlRegistry,
        _event: <ClientWlRegistry as ::wayland_client::Proxy>::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qh: &QueueHandle<BridgeState>,
    ) {}
}

// ─── GlobalDispatch ─────────────────────────────────────────────────────────

impl GlobalDispatch<WlCompositor, CompositorGlobal> for BridgeState {
    fn bind(
        _state: &mut Self, _dh: &DisplayHandle, _client: &Client,
        resource: wayland_server::New<WlCompositor>, _data: &CompositorGlobal, init: &mut DataInit<'_, Self>,
    ) { info!("Client bound wl_compositor"); init.init(resource, ()); }
}

impl GlobalDispatch<WlSubcompositor, SubcompositorGlobal> for BridgeState {
    fn bind(
        _state: &mut Self, _dh: &DisplayHandle, _client: &Client,
        resource: wayland_server::New<WlSubcompositor>, _data: &SubcompositorGlobal, init: &mut DataInit<'_, Self>,
    ) { info!("Client bound wl_subcompositor"); init.init(resource, ()); }
}

impl GlobalDispatch<WlShm, ShmGlobal> for BridgeState {
    fn bind(
        _state: &mut Self, _dh: &DisplayHandle, _client: &Client,
        resource: wayland_server::New<WlShm>, _data: &ShmGlobal, init: &mut DataInit<'_, Self>,
    ) { info!("Client bound wl_shm"); init.init(resource, ()); }
}

impl GlobalDispatch<WlDataDeviceManager, DataDeviceManagerGlobal> for BridgeState {
    fn bind(
        _state: &mut Self, _dh: &DisplayHandle, _client: &Client,
        resource: wayland_server::New<WlDataDeviceManager>, _data: &DataDeviceManagerGlobal, init: &mut DataInit<'_, Self>,
    ) { info!("Client bound wl_data_device_manager"); init.init(resource, ()); }
}

impl GlobalDispatch<WlSeat, SeatGlobal> for BridgeState {
    fn bind(
        _state: &mut Self, _dh: &DisplayHandle, _client: &Client,
        resource: wayland_server::New<WlSeat>, _data: &SeatGlobal, init: &mut DataInit<'_, Self>,
    ) { info!("Client bound wl_seat"); init.init(resource, ()); }
}

impl GlobalDispatch<WlOutput, OutputGlobal> for BridgeState {
    fn bind(
        _state: &mut Self, _dh: &DisplayHandle, _client: &Client,
        resource: wayland_server::New<WlOutput>, _data: &OutputGlobal, init: &mut DataInit<'_, Self>,
    ) { info!("Client bound wl_output"); init.init(resource, ()); }
}

impl GlobalDispatch<server_shell::ZwlrLayerShellV1, LayerShellGlobal> for BridgeState {
    fn bind(
        _state: &mut Self, _dh: &DisplayHandle, _client: &Client,
        resource: wayland_server::New<server_shell::ZwlrLayerShellV1>, _data: &LayerShellGlobal, init: &mut DataInit<'_, Self>,
    ) { info!("Client bound zwlr_layer_shell_v1"); init.init(resource, LayerShellGlobal); }
}

// ─── Dispatch for server protocols ──────────────────────────────────────────

impl Dispatch<WlCompositor, ()> for BridgeState {
    fn request(
        _state: &mut Self, _client: &Client, _resource: &WlCompositor,
        request: <WlCompositor as Resource>::Request, _data: &(), _dh: &DisplayHandle, init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wayland_server::protocol::wl_compositor::Request::CreateSurface { id } => { info!("Client creates wl_surface"); init.init(id, ()); }
            wayland_server::protocol::wl_compositor::Request::CreateRegion { id } => { info!("Client creates wl_region"); init.init(id, ()); }
            _ => {}
        }
    }
}

impl Dispatch<WlRegion, ()> for BridgeState {
    fn request(
        _state: &mut Self, _client: &Client, _resource: &WlRegion,
        _request: <WlRegion as Resource>::Request, _data: &(), _dh: &DisplayHandle, _init: &mut DataInit<'_, Self>,
    ) {}
}

impl Dispatch<WlSubcompositor, ()> for BridgeState {
    fn request(
        _state: &mut Self, _client: &Client, _resource: &WlSubcompositor,
        _request: <WlSubcompositor as Resource>::Request, _data: &(), _dh: &DisplayHandle, _init: &mut DataInit<'_, Self>,
    ) { info!("wl_subcompositor request"); }
}

impl Dispatch<WlShm, ()> for BridgeState {
    fn request(
        _state: &mut Self, _client: &Client, _resource: &WlShm,
        _request: <WlShm as Resource>::Request, _data: &(), _dh: &DisplayHandle, _init: &mut DataInit<'_, Self>,
    ) {}
}

impl Dispatch<WlDataDeviceManager, ()> for BridgeState {
    fn request(
        _state: &mut Self, _client: &Client, _resource: &WlDataDeviceManager,
        _request: <WlDataDeviceManager as Resource>::Request, _data: &(), _dh: &DisplayHandle, _init: &mut DataInit<'_, Self>,
    ) { info!("wl_data_device_manager request"); }
}

impl Dispatch<WlSeat, ()> for BridgeState {
    fn request(
        _state: &mut Self, _client: &Client, _resource: &WlSeat,
        _request: <WlSeat as Resource>::Request, _data: &(), _dh: &DisplayHandle, _init: &mut DataInit<'_, Self>,
    ) { info!("wl_seat request"); }
}

impl Dispatch<WlOutput, ()> for BridgeState {
    fn request(
        _state: &mut Self, _client: &Client, _resource: &WlOutput,
        _request: <WlOutput as Resource>::Request, _data: &(), _dh: &DisplayHandle, _init: &mut DataInit<'_, Self>,
    ) {}
}

impl Dispatch<WlSurface, ()> for BridgeState {
    fn request(
        _state: &mut Self, _client: &Client, _resource: &WlSurface,
        request: <WlSurface as Resource>::Request, _data: &(), _dh: &DisplayHandle, _init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wayland_server::protocol::wl_surface::Request::Attach { buffer, x, y, .. } => {
                info!("wl_surface.attach(buffer={}, x={}, y={})", buffer.is_some(), x, y);
            }
            wayland_server::protocol::wl_surface::Request::Commit { .. } => {}
            wayland_server::protocol::wl_surface::Request::Damage { x, y, width, height, .. } => {
                info!("wl_surface.damage({},{},{},{})", x, y, width, height);
            }
            _ => {}
        }
    }
}

// ═══ Server-side zwlr_layer_shell_v1 dispatch ═══

impl Dispatch<server_shell::ZwlrLayerShellV1, LayerShellGlobal> for BridgeState {
    fn request(
        state: &mut Self, _client: &Client, _resource: &server_shell::ZwlrLayerShellV1,
        request: <server_shell::ZwlrLayerShellV1 as Resource>::Request,
        _data: &LayerShellGlobal, _dh: &DisplayHandle, init: &mut DataInit<'_, Self>,
    ) {
        match request {
            server_shell::Request::GetLayerSurface { id, surface: _server_surface, output: _output, layer, namespace } => {
                state.layer_surface_count += 1;
                let n = state.layer_surface_count;
                let layer_val: u32 = layer.into();

                info!("⭐ get_layer_surface #{}: layer={}, namespace='{}'", n, layer_val, namespace);

                if let (Some(ref compositor), Some(ref layer_shell)) = (state.client_compositor, state.client_layer_shell) {
                    let qh = state.client_event_queue.handle();

                    // Create client-side wl_surface
                    match compositor.send_constructor(
                        wayland_client::protocol::wl_compositor::Request::CreateSurface {
                            id: wayland_client::backend::ProxyId::new(0).into(),
                        },
                        Arc::new(ClientSurfaceData) as Arc<dyn ObjectData>,
                        &qh,
                    ) {
                        Ok(client_wsurface) => {
                            info!("  ✅ Created client-side wl_surface on Mutter");

                            // Create client-side layer surface
                            match layer_shell.send_constructor(
                                client_shell::Request::GetLayerSurface {
                                    id: wayland_client::backend::ProxyId::new(0).into(),
                                    surface: client_wsurface,
                                    output: state.client_outputs.first().cloned(),
                                    layer,
                                    namespace: namespace.clone(),
                                },
                                Arc::new(ClientLayerSurfaceData) as Arc<dyn ObjectData>,
                                &qh,
                            ) {
                                Ok(client_lsurface) => {
                                    info!("  ✅ Created client-side layer surface on Mutter!");
                                    let sdata = LayerSurfaceData {
                                        namespace: namespace.to_string(), layer: layer_val,
                                        client_id: _client.id(), client_layer_surface: Some(client_lsurface),
                                    };
                                    let srv = init.init(id, sdata);
                                    srv.configure(0, 0, 1);
                                    info!("  ↪ configured + linked to Mutter!");
                                }
                                Err(e) => {
                                    warn!("  ⚠ Failed client layer surface: {e}");
                                    let sdata = LayerSurfaceData {
                                        namespace: namespace.to_string(), layer: layer_val,
                                        client_id: _client.id(), client_layer_surface: None,
                                    };
                                    let srv = init.init(id, sdata);
                                    srv.configure(0, 0, 1);
                                }
                            }
                        }
                        Err(e) => {
                            warn!("  ⚠ Failed client wl_surface: {e}");
                            let sdata = LayerSurfaceData {
                                namespace: namespace.to_string(), layer: layer_val,
                                client_id: _client.id(), client_layer_surface: None,
                            };
                            let srv = init.init(id, sdata);
                            srv.configure(0, 0, 1);
                        }
                    }
                } else {
                    warn!("  ⚠ No client compositor/layer_shell connected to Mutter yet!");
                    let sdata = LayerSurfaceData {
                        namespace: namespace.to_string(), layer: layer_val,
                        client_id: _client.id(), client_layer_surface: None,
                    };
                    let srv = init.init(id, sdata);
                    srv.configure(0, 0, 1);
                }
            }
            server_shell::Request::Destroy { .. } => info!("zwlr_layer_shell_v1.destroy"),
            _ => warn!("Unhandled zwlr_layer_shell_v1 request"),
        }
    }
}

// ═══ Server-side zwlr_layer_surface_v1 dispatch ═══

impl Dispatch<server_surface::ZwlrLayerSurfaceV1, LayerSurfaceData> for BridgeState {
    fn request(
        state: &mut Self, _client: &Client, _resource: &server_surface::ZwlrLayerSurfaceV1,
        request: <server_surface::ZwlrLayerSurfaceV1 as Resource>::Request,
        data: &LayerSurfaceData, _dh: &DisplayHandle, _init: &mut DataInit<'_, Self>,
    ) {
        let n = state.layer_surface_count;

        let forward = |r: client_surface::Request| -> Option<()> {
            let cl = data.client_layer_surface.as_ref()?;
            let qh = state.client_event_queue.handle();
            cl.send_constructor(r, Arc::new(ClientLayerSurfaceData) as Arc<dyn ObjectData>, &qh).ok()
        };

        match request {
            server_surface::Request::SetSize { width, height } => {
                info!("  layer_surface #{} set_size({}x{})", n, width, height);
                forward(client_surface::Request::SetSize { width, height });
            }
            server_surface::Request::SetAnchor { anchor } => {
                info!("  layer_surface #{} set_anchor(0x{:x})", n, u32::from(anchor.clone()));
                forward(client_surface::Request::SetAnchor { anchor });
            }
            server_surface::Request::SetExclusiveZone { zone } => {
                info!("  layer_surface #{} set_exclusive_zone({})", n, zone);
                forward(client_surface::Request::SetExclusiveZone { zone });
            }
            server_surface::Request::SetMargin { top, right, bottom, left } => {
                info!("  layer_surface #{} set_margin({},{},{},{})", n, top, right, bottom, left);
                forward(client_surface::Request::SetMargin { top, right, bottom, left });
            }
            server_surface::Request::SetKeyboardInteractivity { keyboard_interactivity } => {
                let ki: u32 = keyboard_interactivity.clone().into();
                info!("  layer_surface #{} set_keyboard_interactivity({})", n, ki);
                forward(client_surface::Request::SetKeyboardInteractivity { keyboard_interactivity });
            }
            server_surface::Request::GetPopup { popup: _, .. } => {
                info!("  layer_surface #{} get_popup (not implemented)", n);
            }
            server_surface::Request::AckConfigure { serial } => {
                info!("  layer_surface #{} ack_configure({})", n, serial);
                forward(client_surface::Request::AckConfigure { serial });
            }
            server_surface::Request::Destroy { .. } => {
                info!("  layer_surface #{} destroy", n);
                forward(client_surface::Request::Destroy {});
            }
            _ => warn!("Unhandled zwlr_layer_surface_v1 request"),
        }
    }
}

// ─── Event loop ─────────────────────────────────────────────────────────────

/// Bind a Mutter global using raw registry `send_constructor` + ObjectData.
fn bind_mutter_global<I>(
    registry: &ClientWlRegistry,
    name: u32,
    version: u32,
    data: impl ObjectData + 'static,
    qh: &QueueHandle<BridgeState>,
) -> Result<I>
where
    I: wayland_client::Proxy + 'static,
{
    let proxy = registry.send_constructor(
        wayland_client::protocol::wl_registry::Request::Bind {
            name,
            id: wayland_client::backend::ProxyId::new(0).into(),
            interface: I::interface().name.to_string(),
            version,
        },
        Arc::new(data) as Arc<dyn ObjectData>,
        qh,
    )?;
    Ok(proxy)
}

pub fn run_event_loop(
    mut display: Display<BridgeState>,
    socket: ListeningSocket,
    _config: Arc<BridgeConfig>,
) -> Result<()> {
    info!("Connecting to Mutter compositor on '{}'...", _config.compositor_display);

    let prev_wl = std::env::var("WAYLAND_DISPLAY").ok();
    std::env::set_var("WAYLAND_DISPLAY", &_config.compositor_display);
    let conn = Connection::connect_to_env().context("Failed to connect to Mutter")?;
    if let Some(p) = prev_wl { std::env::set_var("WAYLAND_DISPLAY", p); }
    else { std::env::remove_var("WAYLAND_DISPLAY"); }

    let (globals, client_queue) = registry_queue_init::<BridgeState>(&conn)
        .context("Failed to init registry with Mutter")?;
    let qh = client_queue.handle();

    let registry: ClientWlRegistry = conn.display().get_registry(&qh, ());

    let contents = globals.contents();
    let guard = contents.lock().unwrap();

    let mut client_compositor: Option<ClientWlCompositor> = None;
    let mut client_layer_shell: Option<client_shell::ZwlrLayerShellV1> = None;
    let mut client_outputs: Vec<ClientWlOutput> = Vec::new();
    let mut client_seat: Option<ClientWlSeat> = None;

    for g in guard.iter() {
        info!("  [Mutter] {} v{} (name={})", g.interface, g.version, g.name);
        match g.interface.as_str() {
            "wl_compositor" => {
                client_compositor = bind_mutter_global(&registry, g.name, g.version.min(5),
                    ClientCompositorData, &qh).ok();
                info!("  ✅ Bound wl_compositor");
            }
            "zwlr_layer_shell_v1" => {
                client_layer_shell = bind_mutter_global(&registry, g.name, g.version.min(4),
                    ClientLayerShellData, &qh).ok();
                info!("  ✅ Bound zwlr_layer_shell_v1");
            }
            "wl_output" => {
                if let Ok(o) = bind_mutter_global::<ClientWlOutput>(&registry, g.name, g.version.min(4),
                    ClientOutputData, &qh) {
                    client_outputs.push(o);
                    info!("  ✅ Bound wl_output #{}", client_outputs.len());
                }
            }
            "wl_seat" => {
                client_seat = bind_mutter_global(&registry, g.name, g.version.min(8),
                    ClientSeatData, &qh).ok();
                info!("  ✅ Bound wl_seat");
            }
            _ => info!("  (not bound) {}", g.interface),
        }
    }
    drop(guard);

    info!("Connected to Mutter: compositor={}, layer_shell={}, outputs={}, seat={}",
        client_compositor.is_some(), client_layer_shell.is_some(),
        client_outputs.len(), client_seat.is_some());

    let mut state = BridgeState {
        layer_surface_count: 0,
        client_conn: conn,
        client_event_queue: client_queue,
        client_compositor, client_layer_shell, client_outputs, client_seat,
        client_registry: Some(registry),
    };

    if let Err(e) = state.client_event_queue.roundtrip(&mut state) {
        error!("Initial client roundtrip error: {e}");
    }

    // Register server globals
    {
        let dh = display.handle();
        dh.create_global::<BridgeState, WlCompositor, CompositorGlobal>(5, CompositorGlobal);
        dh.create_global::<BridgeState, WlSubcompositor, SubcompositorGlobal>(1, SubcompositorGlobal);
        dh.create_global::<BridgeState, WlShm, ShmGlobal>(1, ShmGlobal);
        dh.create_global::<BridgeState, WlDataDeviceManager, DataDeviceManagerGlobal>(3, DataDeviceManagerGlobal);
        dh.create_global::<BridgeState, WlSeat, SeatGlobal>(8, SeatGlobal);
        dh.create_global::<BridgeState, WlOutput, OutputGlobal>(4, OutputGlobal);
        dh.create_global::<BridgeState, server_shell::ZwlrLayerShellV1, LayerShellGlobal>(4, LayerShellGlobal);
        info!("Registered 7 server globals");
    }

    display.flush_clients()?;
    info!("⏳ Bridge Phase 3b running. Accepting clients and proxying to Mutter...");

    loop {
        match socket.accept() {
            Ok(Some(stream)) => {
                info!("New client connection accepted");
                if let Err(e) = display.handle().insert_client(stream, Arc::new(BridgeClientData) as Arc<dyn ClientData>) {
                    error!("Insert client: {e}");
                }
            }
            Ok(None) => {}
            Err(e) => error!("Accept: {e}"),
        }

        if let Err(e) = display.dispatch_clients(&mut state) { error!("Server dispatch: {e}"); break; }
        if let Err(e) = display.flush_clients() { error!("Server flush: {e}"); break; }
        if let Err(e) = state.client_event_queue.dispatch_pending(&mut state) { error!("Client dispatch: {e}"); break; }
        if let Err(e) = state.client_conn.flush() { error!("Client flush: {e}"); break; }

        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    info!("Event loop exited");
    Ok(())
}

struct BridgeClientData;
impl ClientData for BridgeClientData {
    fn initialized(&self, id: ClientId) { info!("Client initialized (id={:?})", id); }
    fn disconnected(&self, id: ClientId, r: DisconnectReason) { info!("Client disconnected (id={:?})", id); }
}
