//! Phase 3b: Proxy bridge with compositor backend.
//!
//! Exposes `zwlr_layer_shell_v1` global and translates intercepted
//! requests to real Mutter (wayland-1) calls via `wayland_client`.

use std::sync::Arc;

use anyhow::{Context, Result};
use log::{error, info, warn};
use wayland_backend::server::{ClientData, ClientId, DisconnectReason};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_compositor::WlCompositor as ClientWlCompositor;
use wayland_client::protocol::wl_output::WlOutput as ClientWlOutput;
use wayland_client::protocol::wl_registry::WlRegistry as ClientWlRegistry;
use wayland_client::protocol::wl_seat::WlSeat as ClientWlSeat;
use wayland_client::protocol::wl_surface::WlSurface as ClientWlSurface;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
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
    Client, DataInit, Display, DisplayHandle, GlobalDispatch,
    ListeningSocket, Resource, WEnum,
};

use crate::BridgeConfig;

// ─── Server global markers ──────────────────────────────────────────────────

pub struct CompositorGlobal;
pub struct SubcompositorGlobal;
pub struct ShmGlobal;
pub struct DataDeviceManagerGlobal;
pub struct SeatGlobal;
pub struct OutputGlobal;
pub struct LayerShellGlobal;

pub struct LayerSurfaceData {
    pub namespace: String,
    pub layer: u32,
    pub client_id: ClientId,
    pub client_layer_surface: Option<client_surface::ZwlrLayerSurfaceV1>,
    pub client_surface: Option<ClientWlSurface>,
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

// ═══ Client Dispatch impls (needed by GlobalList::bind) ═════════════════════

impl Dispatch<ClientWlRegistry, GlobalListContents> for BridgeState {
    fn event(
        _state: &mut Self, _registry: &ClientWlRegistry,
        _event: <ClientWlRegistry as Proxy>::Event, _data: &GlobalListContents,
        _conn: &Connection, _qh: &QueueHandle<BridgeState>,
    ) {}
}

impl Dispatch<ClientWlCompositor, ()> for BridgeState {
    fn event(
        _state: &mut Self, _proxy: &ClientWlCompositor,
        _event: <ClientWlCompositor as Proxy>::Event, _data: &(),
        _conn: &Connection, _qh: &QueueHandle<BridgeState>,
    ) {}
}

impl Dispatch<ClientWlOutput, ()> for BridgeState {
    fn event(
        _state: &mut Self, _proxy: &ClientWlOutput,
        _event: <ClientWlOutput as Proxy>::Event, _data: &(),
        _conn: &Connection, _qh: &QueueHandle<BridgeState>,
    ) {}
}

impl Dispatch<ClientWlSeat, ()> for BridgeState {
    fn event(
        _state: &mut Self, _proxy: &ClientWlSeat,
        _event: <ClientWlSeat as Proxy>::Event, _data: &(),
        _conn: &Connection, _qh: &QueueHandle<BridgeState>,
    ) {}
}

impl Dispatch<client_shell::ZwlrLayerShellV1, ()> for BridgeState {
    fn event(
        _state: &mut Self, _proxy: &client_shell::ZwlrLayerShellV1,
        _event: <client_shell::ZwlrLayerShellV1 as Proxy>::Event, _data: &(),
        _conn: &Connection, _qh: &QueueHandle<BridgeState>,
    ) {}
}

impl Dispatch<ClientWlSurface, ()> for BridgeState {
    fn event(
        _state: &mut Self, _proxy: &ClientWlSurface,
        _event: <ClientWlSurface as Proxy>::Event, _data: &(),
        _conn: &Connection, _qh: &QueueHandle<BridgeState>,
    ) {}
}

impl Dispatch<client_surface::ZwlrLayerSurfaceV1, ()> for BridgeState {
    fn event(
        _state: &mut Self, _proxy: &client_surface::ZwlrLayerSurfaceV1,
        _event: <client_surface::ZwlrLayerSurfaceV1 as Proxy>::Event, _data: &(),
        _conn: &Connection, _qh: &QueueHandle<BridgeState>,
    ) {}
}

// ═══ Server GlobalDispatch ══════════════════════════════════════════════════

impl GlobalDispatch<WlCompositor, CompositorGlobal> for BridgeState {
    fn bind(
        _s: &mut Self, _dh: &DisplayHandle, _c: &Client, r: wayland_server::New<WlCompositor>,
        _d: &CompositorGlobal, init: &mut DataInit<'_, Self>,
    ) { info!("[sv] wl_compositor v5"); init.init(r, ()); }
}

impl GlobalDispatch<WlSubcompositor, SubcompositorGlobal> for BridgeState {
    fn bind(
        _s: &mut Self, _dh: &DisplayHandle, _c: &Client, r: wayland_server::New<WlSubcompositor>,
        _d: &SubcompositorGlobal, init: &mut DataInit<'_, Self>,
    ) { info!("[sv] wl_subcompositor v1"); init.init(r, ()); }
}

impl GlobalDispatch<WlShm, ShmGlobal> for BridgeState {
    fn bind(
        _s: &mut Self, _dh: &DisplayHandle, _c: &Client, r: wayland_server::New<WlShm>,
        _d: &ShmGlobal, init: &mut DataInit<'_, Self>,
    ) { info!("[sv] wl_shm v1"); init.init(r, ()); }
}

impl GlobalDispatch<WlDataDeviceManager, DataDeviceManagerGlobal> for BridgeState {
    fn bind(
        _s: &mut Self, _dh: &DisplayHandle, _c: &Client, r: wayland_server::New<WlDataDeviceManager>,
        _d: &DataDeviceManagerGlobal, init: &mut DataInit<'_, Self>,
    ) { info!("[sv] wl_data_device_manager v3"); init.init(r, ()); }
}

impl GlobalDispatch<WlSeat, SeatGlobal> for BridgeState {
    fn bind(
        _s: &mut Self, _dh: &DisplayHandle, _c: &Client, r: wayland_server::New<WlSeat>,
        _d: &SeatGlobal, init: &mut DataInit<'_, Self>,
    ) { info!("[sv] wl_seat v8"); init.init(r, ()); }
}

impl GlobalDispatch<WlOutput, OutputGlobal> for BridgeState {
    fn bind(
        _s: &mut Self, _dh: &DisplayHandle, _c: &Client, r: wayland_server::New<WlOutput>,
        _d: &OutputGlobal, init: &mut DataInit<'_, Self>,
    ) {
        info!("[sv] wl_output v4");
        use wayland_server::protocol::wl_output::{Mode, Subpixel, Transform};
        let o = init.init(r, ());
        o.geometry(0, 0, 350, 200, Subpixel::Unknown,
            "unknown".to_string(), "unknown".to_string(), Transform::Normal);
        o.mode(Mode::Current | Mode::Preferred, 1920, 1080, 60000);
        o.scale(1);
        o.name("eDP-1".to_string());
        o.description("Dummy bridge output".to_string());
        o.done();
    }
}

impl GlobalDispatch<server_shell::ZwlrLayerShellV1, LayerShellGlobal> for BridgeState {
    fn bind(
        _s: &mut Self, _dh: &DisplayHandle, _c: &Client,
        r: wayland_server::New<server_shell::ZwlrLayerShellV1>,
        _d: &LayerShellGlobal, init: &mut DataInit<'_, Self>,
    ) { info!("[sv] zwlr_layer_shell_v1 v4"); init.init(r, LayerShellGlobal); }
}

// ═══ Server Dispatch for protocol requests ══════════════════════════════════

impl wayland_server::Dispatch<WlCompositor, ()> for BridgeState {
    fn request(
        _s: &mut Self, _c: &Client, _r: &WlCompositor,
        req: <WlCompositor as Resource>::Request, _d: &(), _dh: &DisplayHandle,
        init: &mut wayland_server::DataInit<'_, Self>,
    ) {
        match req {
            wayland_server::protocol::wl_compositor::Request::CreateSurface { id } => { init.init(id, ()); }
            wayland_server::protocol::wl_compositor::Request::CreateRegion { id } => { init.init(id, ()); }
            _ => {}
        }
    }
}

impl wayland_server::Dispatch<WlRegion, ()> for BridgeState {
    fn request(
        _s: &mut Self, _c: &Client, _r: &WlRegion,
        _req: <WlRegion as Resource>::Request, _d: &(), _dh: &DisplayHandle,
        _init: &mut wayland_server::DataInit<'_, Self>,
    ) {}
}

impl wayland_server::Dispatch<WlSubcompositor, ()> for BridgeState {
    fn request(
        _s: &mut Self, _c: &Client, _r: &WlSubcompositor,
        _req: <WlSubcompositor as Resource>::Request, _d: &(), _dh: &DisplayHandle,
        _init: &mut wayland_server::DataInit<'_, Self>,
    ) {}
}

impl wayland_server::Dispatch<WlShm, ()> for BridgeState {
    fn request(
        _s: &mut Self, _c: &Client, _r: &WlShm,
        _req: <WlShm as Resource>::Request, _d: &(), _dh: &DisplayHandle,
        _init: &mut wayland_server::DataInit<'_, Self>,
    ) {}
}

impl wayland_server::Dispatch<WlDataDeviceManager, ()> for BridgeState {
    fn request(
        _s: &mut Self, _c: &Client, _r: &WlDataDeviceManager,
        _req: <WlDataDeviceManager as Resource>::Request, _d: &(), _dh: &DisplayHandle,
        _init: &mut wayland_server::DataInit<'_, Self>,
    ) {}
}

impl wayland_server::Dispatch<WlSeat, ()> for BridgeState {
    fn request(
        _s: &mut Self, _c: &Client, _r: &WlSeat,
        _req: <WlSeat as Resource>::Request, _d: &(), _dh: &DisplayHandle,
        _init: &mut wayland_server::DataInit<'_, Self>,
    ) {}
}

impl wayland_server::Dispatch<WlOutput, ()> for BridgeState {
    fn request(
        _s: &mut Self, _c: &Client, _r: &WlOutput,
        _req: <WlOutput as Resource>::Request, _d: &(), _dh: &DisplayHandle,
        _init: &mut wayland_server::DataInit<'_, Self>,
    ) {}
}

impl wayland_server::Dispatch<WlSurface, ()> for BridgeState {
    fn request(
        _s: &mut Self, _c: &Client, _r: &WlSurface,
        _req: <WlSurface as Resource>::Request, _d: &(), _dh: &DisplayHandle,
        _init: &mut wayland_server::DataInit<'_, Self>,
    ) {}
}

// ═══ Forwarding ObjectData (no events expected from Mutter) ═════════════════

struct FwdObjectData;
impl wayland_client::backend::ObjectData for FwdObjectData {
    fn event(
        self: Arc<Self>,
        _backend: &wayland_client::backend::Backend,
        _msg: wayland_client::backend::protocol::Message<
            wayland_backend::client::ObjectId, std::os::unix::io::OwnedFd,
        >,
    ) -> Option<Arc<dyn wayland_client::backend::ObjectData>> { None }
    fn destroyed(&self, _id: wayland_backend::client::ObjectId) {}
    fn debug(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FwdObjectData")
    }
}

// ═══ Server-side zwlr_layer_shell_v1 dispatch → forward to Mutter ═══

impl wayland_server::Dispatch<server_shell::ZwlrLayerShellV1, LayerShellGlobal> for BridgeState {
    fn request(
        state: &mut Self, _client: &Client, _resource: &server_shell::ZwlrLayerShellV1,
        request: <server_shell::ZwlrLayerShellV1 as Resource>::Request,
        _data: &LayerShellGlobal, _dh: &DisplayHandle, init: &mut wayland_server::DataInit<'_, Self>,
    ) {
        match request {
            server_shell::Request::GetLayerSurface {
                id, surface: _server_surface, output: _output, layer, namespace,
            } => {
                state.layer_surface_count += 1;
                let n = state.layer_surface_count;
                let layer_val: u32 = layer.clone().into();
                info!("⭐ get_layer_surface #{}: layer={}, namespace='{}'", n, layer_val, namespace);

                if let (Some(compositor), Some(layer_shell)) =
                    (state.client_compositor.as_ref(), state.client_layer_shell.as_ref())
                {
                    // 1) Create client-side wl_surface
                    let client_surf: Result<ClientWlSurface, _> = compositor.send_constructor(
                        wayland_client::protocol::wl_compositor::Request::CreateSurface {},
                        Arc::new(FwdObjectData) as Arc<dyn wayland_client::backend::ObjectData>,
                    );
                    match client_surf {
                        Ok(client_surf) => {
                            info!("  ✅ Created wl_surface on Mutter");

                            // Convert server Layer → client Layer via raw value
                            let client_layer = match layer_val {
                                0 => WEnum::Value(client_shell::Layer::Background),
                                1 => WEnum::Value(client_shell::Layer::Bottom),
                                2 => WEnum::Value(client_shell::Layer::Top),
                                _ => WEnum::Value(client_shell::Layer::Overlay),
                            };

                            let client_lsurf: Result<client_surface::ZwlrLayerSurfaceV1, _> = layer_shell.send_constructor(
                                client_shell::Request::GetLayerSurface {
                                    surface: client_surf.clone(),
                                    output: state.client_outputs.first().cloned(),
                                    layer: client_layer,
                                    namespace: namespace.clone(),
                                },
                                Arc::new(FwdObjectData) as Arc<dyn wayland_client::backend::ObjectData>,
                            );

                            match client_lsurf {
                                Ok(client_lsurface) => {
                                    info!("  ✅ Created layer surface on Mutter!");
                                    let lsd = LayerSurfaceData {
                                        namespace: namespace.to_string(), layer: layer_val,
                                        client_id: _client.id(),
                                        client_layer_surface: Some(client_lsurface),
                                        client_surface: Some(client_surf),
                                    };
                                    let srv = init.init(id, lsd);
                                    srv.configure(0, 0, 1);
                                    info!("  ↪ configured + linked!");
                                }
                                Err(e) => {
                                    warn!("  ⚠ client layer surf: {e}");
                                    let lsd = LayerSurfaceData {
                                        namespace: namespace.to_string(), layer: layer_val,
                                        client_id: _client.id(), client_layer_surface: None,
                                        client_surface: None,
                                    };
                                    let srv = init.init(id, lsd);
                                    srv.configure(0, 0, 1);
                                }
                            }
                        }
                        Err(e) => {
                            warn!("  ⚠ client wl_surface: {e}");
                            let lsd = LayerSurfaceData {
                                namespace: namespace.to_string(), layer: layer_val,
                                client_id: _client.id(), client_layer_surface: None,
                                client_surface: None,
                            };
                            let srv = init.init(id, lsd);
                            srv.configure(0, 0, 1);
                        }
                    }
                } else {
                    warn!("  ⚠ No Mutter: comp={}, ls={}",
                        state.client_compositor.is_some(), state.client_layer_shell.is_some());
                    let lsd = LayerSurfaceData {
                        namespace: namespace.to_string(), layer: layer_val,
                        client_id: _client.id(), client_layer_surface: None,
                        client_surface: None,
                    };
                    let srv = init.init(id, lsd);
                    srv.configure(0, 0, 1);
                }
            }
            server_shell::Request::Destroy { .. } => info!("zwlr_layer_shell_v1.destroy"),
            _ => warn!("Unhandled layer_shell request"),
        }
    }
}

// ═══ Server-side zwlr_layer_surface_v1 → forward to Mutter ═══

impl wayland_server::Dispatch<server_surface::ZwlrLayerSurfaceV1, LayerSurfaceData> for BridgeState {
    fn request(
        state: &mut Self, _client: &Client, _resource: &server_surface::ZwlrLayerSurfaceV1,
        request: <server_surface::ZwlrLayerSurfaceV1 as Resource>::Request,
        data: &LayerSurfaceData, _dh: &DisplayHandle, _init: &mut wayland_server::DataInit<'_, Self>,
    ) {
        let n = state.layer_surface_count;

        // Helper: forward request to client-side (Mutter) surface
        // Use send_request for non-constructor requests (SetSize, Destroy, etc.)
        let fwd = |req: client_surface::Request| {
            if let Some(ref cl) = data.client_layer_surface {
                let _: Result<(), _> = cl.send_request(req);
            }
        };

        match request {
            server_surface::Request::SetSize { width, height } => {
                info!("  #{} set_size({}×{})", n, width, height);
                fwd(client_surface::Request::SetSize { width, height });
            }
            server_surface::Request::SetAnchor { anchor } => {
                match anchor {
                    WEnum::Value(a) => {
                        let raw: u32 = a.into();
                        info!("  #{} set_anchor(0x{:x})", n, raw);
                        // Forward as raw value to avoid type mismatch between server/client Anchor
                        fwd(client_surface::Request::SetAnchor {
                            anchor: WEnum::Unknown(raw),
                        });
                    }
                    WEnum::Unknown(v) => {
                        info!("  #{} set_anchor(unknown {})", n, v);
                        fwd(client_surface::Request::SetAnchor {
                            anchor: WEnum::Unknown(v),
                        });
                    }
                }
            }
            server_surface::Request::SetExclusiveZone { zone } => {
                info!("  #{} set_exclusive_zone({})", n, zone);
                fwd(client_surface::Request::SetExclusiveZone { zone });
            }
            server_surface::Request::SetMargin { top, right, bottom, left } => {
                info!("  #{} set_margin({},{},{},{})", n, top, right, bottom, left);
                fwd(client_surface::Request::SetMargin { top, right, bottom, left });
            }
            server_surface::Request::SetKeyboardInteractivity { keyboard_interactivity } => {
                match keyboard_interactivity {
                    WEnum::Value(ki) => {
                        let raw: u32 = ki.into();
                        info!("  #{} set_keyboard_interactivity({})", n, raw);
                        fwd(client_surface::Request::SetKeyboardInteractivity {
                            keyboard_interactivity: WEnum::Unknown(raw),
                        });
                    }
                    WEnum::Unknown(v) => {
                        info!("  #{} set_keyboard_interactivity(unknown {})", n, v);
                        fwd(client_surface::Request::SetKeyboardInteractivity {
                            keyboard_interactivity: WEnum::Unknown(v),
                        });
                    }
                }
            }
            server_surface::Request::GetPopup { popup: _, .. } => {
                info!("  #{} get_popup (NYI)", n);
            }
            server_surface::Request::AckConfigure { serial } => {
                info!("  #{} ack_configure({})", n, serial);
                fwd(client_surface::Request::AckConfigure { serial });
            }
            server_surface::Request::Destroy { .. } => {
                info!("  #{} destroy", n);
                fwd(client_surface::Request::Destroy {});
            }
            _ => warn!("Unhandled layer_surface request"),
        }
    }
}

// ─── Event loop ─────────────────────────────────────────────────────────────

pub fn run_event_loop(
    mut display: Display<BridgeState>,
    socket: ListeningSocket,
    _config: Arc<BridgeConfig>,
) -> Result<()> {
    info!("Connecting to Mutter on '{}'...", _config.compositor_display);

    let prev = std::env::var("WAYLAND_DISPLAY").ok();
    std::env::set_var("WAYLAND_DISPLAY", &_config.compositor_display);
    let conn = Connection::connect_to_env().context("Failed connect to Mutter")?;
    if let Some(p) = prev { std::env::set_var("WAYLAND_DISPLAY", p); }
    else { std::env::remove_var("WAYLAND_DISPLAY"); }

    let (globals, client_queue) = registry_queue_init::<BridgeState>(&conn)
        .context("Failed registry init")?;
    let qh = client_queue.handle();

    // Bind Mutter globals via GlobalList (needs Dispatch<I,()> impls)
    let c_compositor: Option<ClientWlCompositor> = globals.bind(&qh, 1..=5, ()).ok();
    let c_layer_shell: Option<client_shell::ZwlrLayerShellV1> = globals.bind(&qh, 1..=4, ()).ok();
    let c_seat: Option<ClientWlSeat> = globals.bind(&qh, 1..=8, ()).ok();
    let c_outputs: Vec<ClientWlOutput> = globals.bind(&qh, 1..=4, ()).ok()
        .map(|o: ClientWlOutput| vec![o]).unwrap_or_default();

    // Get registry for potential use
    info!("Mutter: compositor={}, layer_shell={}, outputs={}, seat={}",
        c_compositor.is_some(), c_layer_shell.is_some(), c_outputs.len(), c_seat.is_some());

    // ── Register server globals ──
    let dh = display.handle();
    dh.create_global::<BridgeState, WlCompositor, CompositorGlobal>(5, CompositorGlobal);
    dh.create_global::<BridgeState, WlSubcompositor, SubcompositorGlobal>(1, SubcompositorGlobal);
    dh.create_global::<BridgeState, WlShm, ShmGlobal>(1, ShmGlobal);
    dh.create_global::<BridgeState, WlDataDeviceManager, DataDeviceManagerGlobal>(3, DataDeviceManagerGlobal);
    dh.create_global::<BridgeState, WlSeat, SeatGlobal>(8, SeatGlobal);
    dh.create_global::<BridgeState, WlOutput, OutputGlobal>(4, OutputGlobal);
    dh.create_global::<BridgeState, server_shell::ZwlrLayerShellV1, LayerShellGlobal>(4, LayerShellGlobal);
    info!("[sv] 7 globals registered");

    display.flush_clients()?;

    let mut state = Box::new(BridgeState {
        layer_surface_count: 0,
        client_conn: conn,
        client_event_queue: client_queue,
        client_compositor: c_compositor,
        client_layer_shell: c_layer_shell,
        client_outputs: c_outputs,
        client_seat: c_seat,
        client_registry: None,
    });

    // Borrow-checking workaround: extract raw pointers for disjoint fields
    let event_queue_ptr: *mut EventQueue<BridgeState> = &mut state.client_event_queue;
    let conn_ptr: *mut Connection = &mut state.client_conn;

    // Initial roundtrip to complete Mutter protocol handshake
    unsafe {
        if let Err(e) = (*event_queue_ptr).roundtrip(&mut *state) {
            error!("Roundtrip: {e}");
        }
    }

    info!("⏳ Bridge Phase 3b running. Accepting clients and proxying to Mutter...");

    loop {
        match socket.accept() {
            Ok(Some(stream)) => {
                info!("[sv] New client");
                if let Err(e) = display.handle().insert_client(
                    stream, Arc::new(BridgeClientData) as Arc<dyn ClientData>,
                ) {
                    error!("[sv] Insert: {e}");
                }
            }
            Ok(None) => {}
            Err(e) => { error!("[sv] Accept: {e}"); break; }
        }

        if let Err(e) = display.dispatch_clients(&mut *state) { error!("[sv] Dispatch: {e}"); break; }
        if let Err(e) = display.flush_clients() { error!("[sv] Flush: {e}"); break; }
        unsafe {
            if let Err(e) = (*event_queue_ptr).dispatch_pending(&mut *state) { error!("[cl] Dispatch: {e}"); break; }
            if let Err(e) = (*conn_ptr).flush() { error!("[cl] Flush: {e}"); break; }
        }

        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    info!("Event loop exited");
    Ok(())
}

struct BridgeClientData;
impl ClientData for BridgeClientData {
    fn initialized(&self, id: ClientId) { info!("[sv] Client init (id={:?})", id); }
    fn disconnected(&self, id: ClientId, _r: DisconnectReason) { info!("[sv] Client disc (id={:?})", id); }
}
