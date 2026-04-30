use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Result};
use calloop::{EventLoop, Interest, Mode};
use calloop::generic::Generic;
use calloop::LoopSignal;
use log::{debug, error, info, warn};

/// Static flag set by signal handlers to request graceful shutdown.
/// Lives in the proxy module so both the binary and tests can read it.
pub static SHUTDOWN_FLAG: AtomicBool = AtomicBool::new(false);

/// Check whether the static shutdown flag has been set (by signal handlers).
pub(crate) fn check_shutdown() -> bool {
    SHUTDOWN_FLAG.load(Ordering::SeqCst)
}

// ─── Wire message ───────────────────────────────────────────────────────────

#[derive(Debug)]
struct RawMsg {
    raw: Vec<u8>,
    fds: Vec<OwnedFd>,
}

impl RawMsg {
    fn object_id(&self) -> u32 {
        u32::from_ne_bytes([self.raw[0], self.raw[1], self.raw[2], self.raw[3]])
    }
    fn opcode(&self) -> u16 {
        let os = u32::from_ne_bytes([self.raw[4], self.raw[5], self.raw[6], self.raw[7]]);
        (os & 0xffff) as u16
    }
    #[allow(dead_code)]
    fn size(&self) -> usize {
        let os = u32::from_ne_bytes([self.raw[4], self.raw[5], self.raw[6], self.raw[7]]);
        (os >> 16) as usize
    }
}

pub fn make_raw(oid: u32, op: u16, pay: &[u8]) -> Vec<u8> {
    let total = 8u32 + pay.len() as u32;
    let mut m = Vec::with_capacity(total as usize);
    m.extend_from_slice(&oid.to_ne_bytes());
    m.extend_from_slice(&((total << 16) | op as u32).to_ne_bytes());
    m.extend_from_slice(pay);
    m
}

/// Send one complete Wayland wire message (data + optional FDs) via sendmsg.
/// Send raw buffer + fds (owned) without requiring a RawMsg reference.
fn send_raw_raw(stream: &UnixStream, raw: Vec<u8>, fds: &[OwnedFd]) -> Result<()> {
    let fd = stream.as_raw_fd();
    let iov = [libc::iovec {
        iov_base: raw.as_ptr() as *mut _,
        iov_len: raw.len(),
    }];

    if fds.is_empty() {
        let mut mhdr: libc::msghdr = unsafe { std::mem::zeroed() };
        mhdr.msg_iov = iov.as_ptr() as *mut _;
        mhdr.msg_iovlen = 1;
        unsafe {
            let n = libc::sendmsg(fd, &mhdr, libc::MSG_NOSIGNAL);
            if n < 0 {
                bail!("sendmsg: {}", std::io::Error::last_os_error());
            }
            if n as usize != raw.len() {
                bail!("sendmsg: short write ({n})");
            }
        }
        return Ok(());
    }

    let fds_raw: Vec<RawFd> = fds.iter().map(|f| f.as_raw_fd()).collect();
    let clen =
        unsafe { libc::CMSG_SPACE((fds_raw.len() * std::mem::size_of::<RawFd>()) as _) as usize };
    let mut cbuf = vec![0u8; clen];
    let mut mhdr: libc::msghdr = unsafe { std::mem::zeroed() };
    mhdr.msg_iov = iov.as_ptr() as *mut _;
    mhdr.msg_iovlen = 1;
    mhdr.msg_control = cbuf.as_mut_ptr() as *mut _;
    mhdr.msg_controllen = clen;

    unsafe {
        let c = libc::CMSG_FIRSTHDR(&mhdr);
        if c.is_null() {
            bail!("CMSG_FIRSTHDR null");
        }
        let c = &mut *c;
        c.cmsg_level = libc::SOL_SOCKET;
        c.cmsg_type = libc::SCM_RIGHTS;
        c.cmsg_len =
            libc::CMSG_LEN((fds_raw.len() * std::mem::size_of::<RawFd>()) as _) as _;
        std::ptr::copy_nonoverlapping(
            fds_raw.as_ptr(),
            libc::CMSG_DATA(c) as *mut RawFd,
            fds_raw.len(),
        );
    }

    unsafe {
        let n = libc::sendmsg(fd, &mhdr, libc::MSG_NOSIGNAL);
        if n < 0 {
            bail!("sendmsg: {}", std::io::Error::last_os_error());
        }
        if n as usize != raw.len() {
            bail!("sendmsg: short write ({n})");
        }
    }
    Ok(())
}

fn send_raw(stream: &UnixStream, msg: &RawMsg) -> Result<()> {
    let fd = stream.as_raw_fd();
    let iov = [libc::iovec {
        iov_base: msg.raw.as_ptr() as *mut _,
        iov_len: msg.raw.len(),
    }];

    if msg.fds.is_empty() {
        let mut mhdr: libc::msghdr = unsafe { std::mem::zeroed() };
        mhdr.msg_iov = iov.as_ptr() as *mut _;
        mhdr.msg_iovlen = 1;
        unsafe {
            let n = libc::sendmsg(fd, &mhdr, libc::MSG_NOSIGNAL);
            if n < 0 {
                bail!("sendmsg: {}", std::io::Error::last_os_error());
            }
            if n as usize != msg.raw.len() {
                bail!("sendmsg: short write ({n})");
            }
        }
        return Ok(());
    }

    let fds_raw: Vec<RawFd> = msg.fds.iter().map(|f| f.as_raw_fd()).collect();
    let clen =
        unsafe { libc::CMSG_SPACE((fds_raw.len() * std::mem::size_of::<RawFd>()) as _) as usize };
    let mut cbuf = vec![0u8; clen];
    let mut mhdr: libc::msghdr = unsafe { std::mem::zeroed() };
    mhdr.msg_iov = iov.as_ptr() as *mut _;
    mhdr.msg_iovlen = 1;
    mhdr.msg_control = cbuf.as_mut_ptr() as *mut _;
    mhdr.msg_controllen = clen;

    unsafe {
        let c = libc::CMSG_FIRSTHDR(&mhdr);
        if c.is_null() {
            bail!("CMSG_FIRSTHDR null");
        }
        let c = &mut *c;
        c.cmsg_level = libc::SOL_SOCKET;
        c.cmsg_type = libc::SCM_RIGHTS;
        c.cmsg_len =
            libc::CMSG_LEN((fds_raw.len() * std::mem::size_of::<RawFd>()) as _) as _;
        std::ptr::copy_nonoverlapping(
            fds_raw.as_ptr(),
            libc::CMSG_DATA(c) as *mut RawFd,
            fds_raw.len(),
        );
    }

    unsafe {
        let n = libc::sendmsg(fd, &mhdr, libc::MSG_NOSIGNAL);
        if n < 0 {
            bail!("sendmsg: {}", std::io::Error::last_os_error());
        }
        if n as usize != msg.raw.len() {
            bail!("sendmsg: short write ({n})");
        }
    }
    Ok(())
}

// ─── Read raw message ───────────────────────────────────────────────────────

/// Read one complete Wayland wire message from the stream.
/// Uses a two-step read: first the 8-byte header (to get the size),
/// then exactly the payload. This ensures we don't consume more than
/// one message per call.
fn read_raw(stream: &UnixStream) -> Result<RawMsg> {
    let fd = stream.as_raw_fd();

    // Step 1: Read 8-byte header (including any FDs)
    let mut hdr_buf = [0u8; 8];
    let mut cbuf = vec![0u8; 4096];
    let mut hdr_iov = [libc::iovec {
        iov_base: hdr_buf.as_mut_ptr() as *mut _,
        iov_len: 8,
    }];
    let mut hdr_msg: libc::msghdr = unsafe { std::mem::zeroed() };
    hdr_msg.msg_iov = hdr_iov.as_mut_ptr();
    hdr_msg.msg_iovlen = 1;
    hdr_msg.msg_control = cbuf.as_mut_ptr() as *mut _;
    hdr_msg.msg_controllen = cbuf.len();

    let n = unsafe { libc::recvmsg(fd, &mut hdr_msg, libc::MSG_WAITALL) };
    if n < 0 {
        let e = std::io::Error::last_os_error();
        if e.kind() == std::io::ErrorKind::Interrupted {
            return read_raw(stream);
        }
        bail!("recvmsg header: {e}");
    }
    if n == 0 {
        bail!("EOF");
    }
    if n < 8 {
        bail!("short header ({n})");
    }

    let os = u32::from_ne_bytes([hdr_buf[4], hdr_buf[5], hdr_buf[6], hdr_buf[7]]);
    let expected = (os >> 16) as usize;
    if expected < 8 {
        bail!("bad msg size {expected}");
    }

    // Step 2: Read the rest of the payload (if any)
    let payload_len = expected - 8;
    let mut raw = Vec::with_capacity(expected);
    raw.extend_from_slice(&hdr_buf);

    if payload_len > 0 {
        let mut pay_buf = vec![0u8; payload_len];
        let mut pay_iov = [libc::iovec {
            iov_base: pay_buf.as_mut_ptr() as *mut _,
            iov_len: payload_len,
        }];
        let mut pay_msg: libc::msghdr = unsafe { std::mem::zeroed() };
        pay_msg.msg_iov = pay_iov.as_mut_ptr();
        pay_msg.msg_iovlen = 1;
        let mut pay_cbuf = vec![0u8; 4096];
        pay_msg.msg_control = pay_cbuf.as_mut_ptr() as *mut _;
        pay_msg.msg_controllen = pay_cbuf.len();

        let n2 = unsafe { libc::recvmsg(fd, &mut pay_msg, libc::MSG_WAITALL) };
        if n2 < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                return read_raw(stream);
            }
            bail!("recvmsg payload: {e}");
        }
        if n2 == 0 {
            bail!("payload EOF");
        }
        if n2 as usize != payload_len {
            bail!("pay: expected {payload_len}, got {n2}");
        }
        raw.extend_from_slice(&pay_buf);

        if pay_msg.msg_controllen > 0 {
            cbuf = pay_cbuf;
        }
    }

    // Extract FDs from cbuf
    let fds = if cbuf.iter().any(|&b| b != 0) {
        let mut fds = Vec::new();
        let mut ch = unsafe { libc::CMSG_FIRSTHDR(&hdr_msg) };
        while !ch.is_null() {
            let c = unsafe { &*ch };
            if c.cmsg_level == libc::SOL_SOCKET && c.cmsg_type == libc::SCM_RIGHTS {
                let payload_len =
                    c.cmsg_len as usize - unsafe { libc::CMSG_LEN(0) as usize };
                let nfds = payload_len / std::mem::size_of::<RawFd>();
                let data = unsafe { libc::CMSG_DATA(ch) as *const RawFd };
                for i in 0..nfds {
                    let raw_fd = unsafe { *data.add(i) };
                    if raw_fd >= 0 {
                        fds.push(unsafe { OwnedFd::from_raw_fd(raw_fd) });
                    }
                }
            }
            ch = unsafe { libc::CMSG_NXTHDR(&hdr_msg, ch) };
        }
        fds
    } else {
        Vec::new()
    };

    Ok(RawMsg { raw, fds })
}

// ─── Protocol injection constants ───────────────────────────────────────────

/// The interface string for wlr-layer-shell.
const LAYER_SHELL_IFACE: &str = "zwlr_layer_shell_v1";
/// Max version we'll advertise for the injected layer shell.
const LAYER_SHELL_VERSION: u32 = 4;
/// Fake global name we inject when the compositor doesn't have layer shell.
/// Using a high value to avoid colliding with real compositor globals.
const FAKE_GLOBAL_LAYER_SHELL: u32 = 43;
/// Starting OID for bridge-managed objects.
/// Set to a reasonably high value to avoid colliding with client's dynamic OIDs
/// but not so high that Mutter rejects it (Mutter caps OIDs internally).
const OID_BASE: u32 = 500;

// ─── State ──────────────────────────────────────────────────────────────────

/// Information about a global as advertised by the compositor.
#[derive(Clone, Debug)]
#[allow(dead_code)]
struct GlobalInfo {
    name: u32,
    interface: String,
    version: u32,
}

/// A tracked fake object managed by the bridge.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct FakeObject {
    /// The OID the client assigned to this object.
    cli_oid: u32,
    /// The interface name (e.g., "zwlr_layer_shell_v1").
    iface: String,
    /// Next available sub-object OID for objects created through this one.
    next_sub_oid: u32,
    /// Any associated data (e.g., namespace for a layer surface).
    data: String,
}

struct Session {
    to_cli: UnixStream,
    to_comp: UnixStream,
    /// Globals we've sniffed from compositor→client registry events.
    comp_globals: Vec<GlobalInfo>,
    /// The client's first registry object ID (set by get_registry).
    cli_reg_id: u32,
    /// The registry object ID on the compositor side (same as cli_reg_id).
    comp_reg_id: u32,
    /// Whether we've finished collecting globals (got a non-registry event).
    globals_collected: bool,
    /// Whether we injected a layer shell global because the compositor didn't have one.
    injected_layer_shell: bool,
    /// Fake objects managed by the bridge (keyed by cli_oid).
    fake_objects: Vec<FakeObject>,
    /// The name of the injected fake global (if any).
    fake_global_name: Option<u32>,
    /// Next available OID for objects created internally.
    next_oid: u32,
    // --- Dynamic multi-monitor ---
    tracked_outputs: Vec<TrackedOutput>,
    output_events_delivered: bool,
    /// Known monitors for input coordinate re-injection.
    monitors: Vec<MonitorInfo>,



    // --- Translation State ---
    cli_xdg_wm_base_id: u32,
    comp_dec_mgr_id: u32,
    comp_dec_mgr_name: u32,
    comp_compositor_id: u32,
    comp_compositor_name: u32,
    /// Translation map: client ID (LayerSurface) -> compositor IDs (XDG Surface, Toplevel)
    translation_map: Vec<TranslationEntry>,
    /// All registry object IDs (client-side) created by this client.
    all_registry_ids: Vec<u32>,
}

#[derive(Clone)]
struct TranslationEntry {
    cli_layer_surface_oid: u32,
    /// The xdg_surface OID the client might have created for the same wl_surface.
    cli_xdg_surface_oid: u32,
    comp_xdg_surface_oid: u32,
    comp_toplevel_oid: u32,
    /// Original client wl_surface ID.
    cli_surface_oid: u32,
    /// Requested output (monitor) OID. 0 means compositor choice.
    requested_output_oid: u32,
    /// Suggested dimensions from compositor.
    pending_width: u32,
    pending_height: u32,
}
/// Tracked output for dynamic multi-monitor support.
#[derive(Debug, Clone)]
struct TrackedOutput {
    comp_oid: u32,
    global_name: u32,
    x: i32,
    y: i32,
}

/// Forward a wl_output global event to the client.
/// Represents a connected output/monitor for input coordinate correction.
#[derive(Debug, Clone)]
struct MonitorInfo {
    comp_oid: u32,
    x: i32,
    y: i32,
    width_mm: i32,
    height_mm: i32,
    scale: i32,
    name: Option<String>,
}

fn forward_output_global(s: &mut Session, name: u32, iface: &str, version: u32) -> Result<()> {
    let evt = make_global_event(s.cli_reg_id, name, iface, version);
    send_raw(&s.to_cli, &RawMsg { raw: evt, fds: Vec::new() })?;
    Ok(())
}

fn sniff_output_event(s: &mut Session, oid: u32, op: u16, pay: &[u8]) {
    // Geometry event (op 4): x, y, width_mm, height_mm (each i32)
    if op == 4 && pay.len() >= 16 {
        let x = i32::from_ne_bytes([pay[0], pay[1], pay[2], pay[3]]);
        let y = i32::from_ne_bytes([pay[4], pay[5], pay[6], pay[7]]);
        let w = i32::from_ne_bytes([pay[8], pay[9], pay[10], pay[11]]);
        let h = i32::from_ne_bytes([pay[12], pay[13], pay[14], pay[15]]);
        if let Some(tracked) = s.tracked_outputs.iter_mut().find(|t| t.comp_oid == oid) {
            if tracked.x != x || tracked.y != y {
                info!("  Output {} geometry changed: ({}x{}) -> ({}x{})", oid, tracked.x, tracked.y, x, y);
                tracked.x = x;
                tracked.y = y;
            }
        }
        // Also update monitor tracking
        if let Some(mon) = s.monitors.iter_mut().find(|m| m.comp_oid == oid) {
            mon.x = x;
            mon.y = y;
            mon.width_mm = w;
            mon.height_mm = h;
        } else {
            // Track this new monitor
            s.monitors.push(MonitorInfo {
                comp_oid: oid,
                x, y, width_mm: w, height_mm: h, scale: 1, name: None,
            });
        }
    }
    // Scale event (op 6): factor (int, actually encoded as wl_fixed_t which is i32)
    if op == 4 && pay.len() >= 4 {
        let slen = u32::from_ne_bytes([pay[0], pay[1], pay[2], pay[3]]) as usize;
        if pay.len() >= 4 + slen {
            let name = String::from_utf8_lossy(&pay[4..4+slen]).trim_end_matches('\0').to_string();
            if let Some(mon) = s.monitors.iter_mut().find(|m| m.comp_oid == oid) {
                mon.name = Some(name);
            }
        }
    }

    if op == 6 && pay.len() >= 4 {
        let scale = i32::from_ne_bytes([pay[0], pay[1], pay[2], pay[3]]);
        if let Some(mon) = s.monitors.iter_mut().find(|m| m.comp_oid == oid) {
            mon.scale = scale;
        }
    }
}

fn handle_dynamic_output_global(s: &mut Session, name: u32, iface: &str, version: u32) -> Result<()> {
    if iface == "wl_output" {
        info!("  Dynamic: new wl_output global name={}, forwarding to client", name);
        forward_output_global(s, name, iface, version)?;
    }
    Ok(())
}


// ─── Public entry point ─────────────────────────────────────────────────────

pub fn run(cfg: BridgeConfig) -> Result<()> {
    run_with_shutdown(cfg, Arc::new(AtomicBool::new(false)))
}

/// Run the bridge with a shared shutdown signal (for daemon mode).
/// When `shutdown` is set to true, the accept loop exits gracefully.
pub fn run_with_shutdown(cfg: BridgeConfig, shutdown: Arc<AtomicBool>) -> Result<()> {
    let rdir = std::env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| "/run/user/1000".to_string());
    let spath = format!("{rdir}/{}", cfg.bridge_display);
    let _ = std::fs::remove_file(&spath);
    let lis = std::os::unix::net::UnixListener::bind(&spath)?;
    std::fs::set_permissions(
        &spath,
        std::os::unix::fs::PermissionsExt::from_mode(0o777),
    )
    .ok();
    lis.set_nonblocking(true)?;
    info!(
        "Listening on {spath}, proxying to {}",
        cfg.compositor_display
    );

    loop {
        // Check graceful shutdown signal (from passed Arc or static signal handler)
        if shutdown.load(Ordering::Relaxed) || check_shutdown() {
            info!("Shutdown signal received, stopping listener");
            break;
        }

        let (cli, _) = match lis.accept() {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(10));
                continue;
            }
            Err(e) => {
                error!("Accept: {e}");
                break;
            }
        };
        cli.set_nonblocking(false)?;
        info!("New client connected");

        let comp_display = cfg.compositor_display.clone();
        let rd2 = rdir.clone();
        let sh = shutdown.clone();

        std::thread::Builder::new()
            .name("client-session".into())
            .spawn(move || {
                let comp = match std::os::unix::net::UnixStream::connect(
                    std::path::Path::new(&rd2).join(&comp_display),
                ) {
                    Ok(s) => s,
                    Err(e) => {
                        error!("Connect: {e}");
                        return;
                    }
                };
                comp.set_nonblocking(false).ok();
                if let Err(e) = session(cli, comp) {
                    // Don't log shutdown-triggered errors as errors
                    if !sh.load(Ordering::Relaxed) {
                        error!("Session error: {e}");
                    }
                }
                info!("Session done");
            })?;
    }

    // Clean up socket file
    let _ = std::fs::remove_file(&spath);
    info!("Socket {spath} removed");
    Ok(())
}

// ─── Config ─────────────────────────────────────────────────────────────────

/// Bridge configuration, loadable from TOML or CLI defaults.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct BridgeConfig {
    /// Unix socket name for the bridge (e.g. "wayland-bridge-0")
    #[serde(default = "default_bridge_display")]
    pub bridge_display: String,
    /// Compositor Wayland display (e.g. "wayland-0")
    #[serde(default = "default_compositor_display")]
    pub compositor_display: String,
    /// Max client threads (0 = unlimited, default)
    #[serde(default)]
    pub max_clients: u32,
    /// Log level filter (overrides env_logger default)
    #[serde(default)]
    pub log_level: Option<String>,
}

fn default_bridge_display() -> String {
    "wayland-bridge-0".into()
}

fn default_compositor_display() -> String {
    std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-0".into())
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            bridge_display: default_bridge_display(),
            compositor_display: default_compositor_display(),
            max_clients: 0,
            log_level: None,
        }
    }
}

impl BridgeConfig {
    /// Load from a TOML file path. Missing fields use defaults.
    pub fn from_file(path: &str) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("Cannot read config '{}': {}", path, e))?;
        let config: BridgeConfig = toml::from_str(&content)?;
        Ok(config)
    }

    /// Merge another config into self (non-default fields override).
    pub fn merge(&mut self, other: &BridgeConfig) {
        if other.bridge_display != default_bridge_display() {
            self.bridge_display.clone_from(&other.bridge_display);
        }
        if other.compositor_display != default_compositor_display() {
            self.compositor_display.clone_from(&other.compositor_display);
        }
        if other.max_clients != 0 {
            self.max_clients = other.max_clients;
        }
        if other.log_level.is_some() {
            self.log_level.clone_from(&other.log_level);
        }
    }
}

// ─── Wire format helpers ────────────────────────────────────────────────────

/// Build a wl_registry.global event for injecting a fake global.
/// Build a wl_registry.global event.
/// Wayland wire format: name(u32) + iface_slen(u32, including null) + iface(padded to 4) + version(u32).
fn make_global_event(reg_oid: u32, name: u32, iface: &str, version: u32) -> Vec<u8> {
    // Null-terminate the string
    let mut iface_b = iface.as_bytes().to_vec();
    iface_b.push(0);
    let slen = iface_b.len() as u32; // includes null
    // Pad to 4 bytes
    let padded = ((slen as usize) + 3) & !3;
    let mut pay = Vec::with_capacity(4 + 4 + padded + 4);
    pay.extend_from_slice(&name.to_ne_bytes());          // 4 bytes: name
    pay.extend_from_slice(&slen.to_ne_bytes());           // 4 bytes: string length
    pay.extend_from_slice(&iface_b);                      // slen bytes: interface name
    while pay.len() < (8 + padded) {
        pay.push(0);
    }
    pay.extend_from_slice(&version.to_ne_bytes());       // 4 bytes: version
    make_raw(reg_oid, 0, &pay) // op 0 = global
}

/// Build a wl_display.delete_id event.
#[allow(dead_code)]
fn make_delete_id(display_oid: u32, id: u32) -> Vec<u8> {
    let pay = id.to_ne_bytes();
    make_raw(display_oid, 1, &pay) // op 1 = delete_id
}

// ─── Session loop ───────────────────────────────────────────────────────────

fn session(to_cli: UnixStream, to_comp: UnixStream) -> Result<()> {
    let rdir = std::env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| "/run/user/1000".to_string());
    let comp_display = std::env::var("WAYLAND_DISPLAY")
        .unwrap_or_else(|_| "wayland-0".to_string());
    // Inject the fake layer_shell global IMMEDIATELY, before any registry is created.
    // This ensures the global is in the client's socket buffer before
    // registry_queue_init starts its round-trip dispatch.

    let comp_path = format!("{rdir}/{comp_display}");
    let mut s = Session {
        to_cli,
        to_comp,
        comp_globals: Vec::new(),
        cli_reg_id: 0,
        comp_reg_id: 0,
        globals_collected: false,
        injected_layer_shell: false,
        fake_objects: Vec::new(),
        fake_global_name: None,
        next_oid: OID_BASE,
        cli_xdg_wm_base_id: 0,
        comp_dec_mgr_id: 0,
        comp_dec_mgr_name: 0,
        comp_compositor_id: 0,
        comp_compositor_name: 0,
        translation_map: Vec::new(),
        tracked_outputs: Vec::new(),
        output_events_delivered: false,
        monitors: Vec::new(),
        all_registry_ids: Vec::new(),

    };

    // Use calloop-based event loop for this session.
    let mut evloop: EventLoop<Session> = EventLoop::try_new()?;
    let signal = Arc::new(std::sync::Mutex::new(Some(evloop.get_signal())));
    let handle = evloop.handle();

    fn stop_session(sig: &Arc<std::sync::Mutex<Option<LoopSignal>>>) {
        if let Some(s) = sig.lock().unwrap().take() {
            s.stop();
        }
    }

    // Register compositor FD
    let comp = s.to_comp.try_clone()?;
    let sig_c = signal.clone();
    handle.insert_source(
        Generic::new(comp, Interest::READ, Mode::Level),
        move |_readiness: calloop::Readiness,
              _fd: &mut calloop::generic::NoIoDrop<UnixStream>,
              session: &mut Session| {
            // Read one message from compositor
            let msg = match read_raw(&session.to_comp) {
                Ok(m) => m,
                Err(e) => {
                    info!("Compositor EOF: {e}");
                    cleanup_session(session);
                    stop_session(&sig_c);
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::ConnectionReset,
                        e.to_string(),
                    ));
                }
            };

            // Collect phase: globals not yet accumulated
            if !session.globals_collected && session.comp_reg_id > 0 {
                if msg.object_id() == session.comp_reg_id && msg.opcode() == 0 {
                    // Registry global event → forward to client and track
                    let _ = send_raw(&session.to_cli, &msg);
                    sniff_global(session, &msg);
                } else {
                    // Non-registry event → globals done, inject layer shell
                    session.globals_collected = true;
                    if let Err(e) = maybe_inject_layer_shell(session) {
                        error!("Inject layer shell: {e}");
                    }
                    info!("globals collected ({}):", session.comp_globals.len());
                    for g in &session.comp_globals {
                        info!("  name={}, iface='{}', v{}", g.name, g.interface, g.version);
                    }
                    // Forward this message to client too
                    let _ = send_raw(&session.to_cli, &msg);
                }
                return Ok(calloop::PostAction::Continue);
            }

            // Normal forwarding
            handle_comp(session, &msg).map_err(|e| {
                std::io::Error::new(std::io::ErrorKind::Other, e.to_string())
            })?;
            Ok(calloop::PostAction::Continue)
        },
    )?;

    // Register client FD
    let cli = s.to_cli.try_clone()?;
    let sig_c = signal.clone();
    handle.insert_source(
        Generic::new(cli, Interest::READ, Mode::Level),
        move |_readiness: calloop::Readiness,
              _fd: &mut calloop::generic::NoIoDrop<UnixStream>,
              session: &mut Session| {
            let msg = match read_raw(&session.to_cli) {
                Ok(m) => m,
                Err(e) => {
                    info!("Client EOF: {e}");
                    cleanup_session(session);
                    stop_session(&sig_c);
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::ConnectionReset,
                        e.to_string(),
                    ));
                }
            };

            // Intercept wl_surface.set_buffer_scale (op 12)
            let oid = msg.object_id();
            let op = msg.opcode();
            if op == 12 {
                if let Some(_entry) = session.translation_map.iter().find(|e| e.cli_surface_oid == oid) {
                    let pay = &msg.raw[8..];
                    if pay.len() >= 4 {
                        let scale = i32::from_ne_bytes([pay[0], pay[1], pay[2], pay[3]]);
                        info!("  Intercepted set_buffer_scale: surface={}, scale={}", oid, scale);
                    }
                }
            }

            handle_cli(session, &msg).map_err(|e| {
                std::io::Error::new(std::io::ErrorKind::Other, e.to_string())
            })?;
            Ok(calloop::PostAction::Continue)
        },
    )?;

    // Dispatch until LoopSignal::stop() is called OR shutdown flag is set
    loop {
        let _ = evloop.dispatch(Some(Duration::from_millis(200)), &mut s);
        
        // Secondary registries are handled via fake objects - no compositor connection to drain
        
        if signal.lock().unwrap().is_none() || SHUTDOWN_FLAG.load(Ordering::Relaxed) {
            break;
        }
    }

    Ok(())
}

// ─── Orphan cleanup ────────────────────────────────────────────────────────

/// Clean up all compositor-side resources when a client disconnects.
/// Sends destroy messages for every bridge-managed object to prevent leaks.
fn cleanup_session(s: &mut Session) {
    let n_entries = s.translation_map.len();
    if n_entries > 0 {
        info!("Cleaning up {} translation entries", n_entries);
    }

    // Destroy toplevels and xdg_surfaces in reverse order (child first)
    for entry in s.translation_map.drain(..).rev() {
        // xdg_toplevel.destroy
        if let Err(e) = send_raw(&s.to_comp, &RawMsg {
            raw: make_raw(entry.comp_toplevel_oid, 0, &[]),
            fds: Vec::new(),
        }) {
            warn!("cleanup: toplevel.destroy failed: {e}");
        }
        // xdg_surface.destroy
        if let Err(e) = send_raw(&s.to_comp, &RawMsg {
            raw: make_raw(entry.comp_xdg_surface_oid, 0, &[]),
            fds: Vec::new(),
        }) {
            warn!("cleanup: xdg_surface.destroy failed: {e}");
        }
    }

    // Cleanup decoration manager bound internally
    if s.comp_dec_mgr_id != 0 {
        // We can't destroy the decoration manager itself (it's compositor-side),
        // but we can forget about it since this session is ending.
        info!("cleanup: decoration manager session done");
    }

    // Cleanup internal compositor binding
    if s.comp_compositor_id != 0 {
        info!("cleanup: internal compositor binding released");
    }

    // Clear all tracked fake objects
    let n_fakes = s.fake_objects.len();
    if n_fakes > 0 {
        info!("Cleanup: discarding {} fake objects", n_fakes);
        s.fake_objects.clear();
    }

    // Reset session tracking state
    s.injected_layer_shell = false;
    s.fake_global_name = None;
}

// ─── Global sniffing & injection ────────────────────────────────────────────

fn sniff_global(s: &mut Session, msg: &RawMsg) {
    let pay = &msg.raw[8..];
    if pay.len() < 12 {
        return;
    }
    let gname = u32::from_ne_bytes([pay[0], pay[1], pay[2], pay[3]]);
    let slen_raw = u32::from_ne_bytes([pay[4], pay[5], pay[6], pay[7]]);
    let slen = slen_raw as usize;
    let iface_end = (8 + slen).min(pay.len());
    let iface = if iface_end > 8 {
        let mut s = String::new();
        for &b in &pay[8..iface_end] {
            if b == 0 {
                break;
            }
            s.push(b as char);
        }
        s
    } else {
        String::new()
    };
    let ver_offset = 8 + ((slen + 3) & !3);
    let version = if ver_offset + 4 <= pay.len() {
        u32::from_ne_bytes([
            pay[ver_offset],
            pay[ver_offset + 1],
            pay[ver_offset + 2],
            pay[ver_offset + 3],
        ])
    } else {
        1
    };
    info!("  collected global: name={gname}, iface='{iface}', v{version}");
    if iface == "zxdg_decoration_manager_v1" {
        s.comp_dec_mgr_name = gname;
    }
    if iface == "wl_compositor" {
        s.comp_compositor_name = gname;
    }
    s.comp_globals.push(GlobalInfo {
        name: gname,
        interface: iface,
        version,
    });
}

/// After globals are collected, inject a fake `zwlr_layer_shell_v1` global
/// if the compositor didn't advertise one.
fn maybe_inject_layer_shell(s: &mut Session) -> Result<()> {
    let has_layer_shell = s.comp_globals.iter().any(|g| g.interface == "zwlr_layer_shell_v1");
    if has_layer_shell {
        info!("compositor has zwlr_layer_shell_v1 — no injection needed");
        return Ok(());
    }

    info!(
        "compositor lacks zwlr_layer_shell_v1 — injecting fake global name={}",
        FAKE_GLOBAL_LAYER_SHELL
    );

    // Send a wl_registry.global event to the client
    let evt = make_global_event(
        s.cli_reg_id,
        FAKE_GLOBAL_LAYER_SHELL,
        LAYER_SHELL_IFACE,
        LAYER_SHELL_VERSION,
    );
    send_raw(
        &s.to_cli,
        &RawMsg {
            raw: evt,
            fds: Vec::new(),
        },
    )?;

    s.injected_layer_shell = true;
    s.fake_global_name = Some(FAKE_GLOBAL_LAYER_SHELL);
    // Also add to our local list for tracking
    s.comp_globals.push(GlobalInfo {
        name: FAKE_GLOBAL_LAYER_SHELL,
        interface: "zwlr_layer_shell_v1".into(),
        version: LAYER_SHELL_VERSION,
    });
    Ok(())
}

// ─── Client messages ────────────────────────────────────────────────────────

fn handle_cli(s: &mut Session, msg: &RawMsg) -> Result<()> {
    let oid = msg.object_id();
    let op = msg.opcode();

    debug!("cli ➔ comp: oid={}, op={}, size={} raw={:02x?}", oid, op, msg.raw.len(), &msg.raw[..msg.raw.len().min(32)]);

    // Intercept xdg_wm_base.get_xdg_surface (op 2)
    if s.cli_xdg_wm_base_id > 0 && oid == s.cli_xdg_wm_base_id && op == 2 {
        let pay = &msg.raw[8..];
        if pay.len() >= 8 {
            let cli_xdg_surf_id = u32::from_ne_bytes([pay[0], pay[1], pay[2], pay[3]]);
            let cli_surf_id = u32::from_ne_bytes([pay[4], pay[5], pay[6], pay[7]]);
            
            // Check if this wl_surface is already managed by a layer_surface
            if let Some(pos) = s.translation_map.iter().position(|e| e.cli_surface_oid == cli_surf_id) {
                info!("  Intercepted xdg_wm_base.get_xdg_surface: Alias cli_xdg_surf={} -> comp_xdg_surf={}", 
                    cli_xdg_surf_id, s.translation_map[pos].comp_xdg_surface_oid);
                
                s.translation_map[pos].cli_xdg_surface_oid = cli_xdg_surf_id;
                
                // Track as fake to intercept its methods (like get_popup)
                s.fake_objects.push(FakeObject {
                    cli_oid: cli_xdg_surf_id,
                    iface: "xdg_surface".into(),
                    next_sub_oid: 0,
                    data: String::new(),
                });
                return Ok(()); // Don't forward to compositor
            }
        }
    }

    // wl_display.get_registry (oid=1, op=1) — forward, then enable sniffing
    if oid == 1 && msg.opcode() == 1 {
        let new_id = if msg.raw.len() >= 12 {
            u32::from_ne_bytes([msg.raw[8], msg.raw[9], msg.raw[10], msg.raw[11]])
        } else {
            2
        };
        info!("get_registry → cli_reg={new_id}, comp_reg={new_id}");
        
        let is_first = s.all_registry_ids.is_empty();
        s.all_registry_ids.push(new_id);
        
        if is_first {
            s.cli_reg_id = new_id;
            // Inject fake layer_shell global BEFORE compositor globals arrive
            info!("Injecting fake layer_shell global (name={}) on registry id={}",
                FAKE_GLOBAL_LAYER_SHELL, new_id);
            let evt = make_global_event(new_id, FAKE_GLOBAL_LAYER_SHELL, LAYER_SHELL_IFACE, LAYER_SHELL_VERSION);
            let _ = send_raw(&s.to_cli, &RawMsg { raw: evt, fds: Vec::new() });
            s.injected_layer_shell = true;
            s.fake_global_name = Some(FAKE_GLOBAL_LAYER_SHELL);
            send_raw(&s.to_comp, msg)?;
            // Yield to let client process fake layer_shell global before sending binds.
            // Without this, the client may send binds that reference globals it hasn't 
            // processed yet, causing Mutter to reject them.
            std::thread::sleep(std::time::Duration::from_millis(5));
        } else {
            // Secondary registry: forward + inject layer_shell.
            // gtk-layer-shell creates its OWN registry via GDK to bind layer_shell.
            // We must inject on ALL registries so GDK finds it.
            info!("Forwarding get_registry for secondary registry id={} + layer_shell injection", new_id);
            let evt = make_global_event(new_id, FAKE_GLOBAL_LAYER_SHELL, LAYER_SHELL_IFACE, LAYER_SHELL_VERSION);
            let _ = send_raw(&s.to_cli, &RawMsg { raw: evt, fds: Vec::new() });
            send_raw(&s.to_comp, msg)?;
            // Yield to let client process fake layer_shell global before sending binds.
            // Without this, the client may send binds that reference globals it hasn't 
            // processed yet, causing Mutter to reject them.
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        return Ok(());
    }

    // Intercept binds for fake layer_shell on ANY registry
    if oid != 1 && msg.opcode() == 0 && msg.raw.len() >= 28 {
        let bind_name = u32::from_ne_bytes([msg.raw[8], msg.raw[9], msg.raw[10], msg.raw[11]]);
        
        // Parse bind details: interface string, version, new_id
        let (bind_iface_str, bind_version, bind_new_id) = parse_bind_payload(&msg.raw[8..]);
        
        // Intercept fake layer_shell bind
        if bind_name == FAKE_GLOBAL_LAYER_SHELL {
            info!("  intercepted fake bind for zwlr_layer_shell_v1 on any registry (oid={})", oid);
            if bind_new_id > 0 && !s.fake_objects.iter().any(|f| f.cli_oid == bind_new_id) {
                s.fake_objects.push(FakeObject { cli_oid: bind_new_id, iface: "zwlr_layer_shell_v1".into(), next_sub_oid: bind_new_id + 1, data: String::new() });
            }
            return Ok(());
        }
        
        // zxdg_output_manager_v1: forward to compositor (Mutter provides it)

        // NON-PRIMARY REGISTRIES: forward binds to compositor with version clamping.
        // (The primary registry goes through handle_bind below, which does the same.)
        if oid != s.cli_reg_id {
            if let Some(iface_str) = &bind_iface_str {
                let known_global = s.comp_globals.iter().find(|g| g.name == bind_name);
                if let Some(global) = known_global {
                    let comp_version = global.version;
                    let mut raw = msg.raw.clone();
                    if bind_version > comp_version {
                        if let Some(ver_off) = find_version_offset_in_bind(&msg.raw[8..]) {
                            raw[8+ver_off..8+ver_off+4].copy_from_slice(&comp_version.to_ne_bytes());
                        }
                    }
                    info!("  bind on registry oid={}: name={}, iface='{}', new_id={}, cli_v={}, comp_v={}",
                        oid, bind_name, iface_str, bind_new_id, bind_version, comp_version);
                    return send_raw_raw(&s.to_comp, raw, &msg.fds);
                } else {
                    info!("  blocking bind to unknown global name={} on registry oid={}", bind_name, oid);
                    return Ok(());
                }
            }
            return Ok(());
        }
    }

    // Registry bind — forward to handle_bind for processing
    if s.cli_reg_id > 0 && oid == s.cli_reg_id && msg.opcode() == 0 {
        return handle_bind(s, msg);
    }

    // Check if this message targets a fake bridge-managed object
    let fake_idx = s.fake_objects.iter().position(|f| f.cli_oid == oid);
    if let Some(idx) = fake_idx {
        // Clone the fake object info so we can drop the immutable borrow
        let fake = s.fake_objects[idx].clone();
        return handle_fake_msg(s, &fake, msg);
    }

    // Debug: log wl_display.sync messages
    if oid == 1 && op == 0 && msg.raw.len() >= 12 {
        let cb_new_id = u32::from_ne_bytes([msg.raw[8], msg.raw[9], msg.raw[10], msg.raw[11]]);
        info!("  wl_display.sync(callback={})", cb_new_id);
    }

    // Everything else: forward raw
    send_raw(&s.to_comp, msg)
}

// ─── Bind parsing helpers ───────────────────────────────────────────────────

/// Parse a bind payload and return (interface_string, version, new_id).
fn parse_bind_payload(pay: &[u8]) -> (Option<String>, u32, u32) {
    if pay.len() < 12 { return (None, 1, 0); }
    let str_len_raw = u32::from_ne_bytes([pay[4], pay[5], pay[6], pay[7]]);
    let slen = str_len_raw as usize;
    let str_padded = (slen + 3) & !3;
    let iface = if slen > 0 && slen < 256 && 8 + slen <= pay.len() {
        Some(String::from_utf8_lossy(&pay[8..8+slen]).trim_end_matches('\0').to_string())
    } else { None };
    let ver_offset = 8 + str_padded;
    let version = if ver_offset + 4 <= pay.len() {
        u32::from_ne_bytes([pay[ver_offset], pay[ver_offset+1], pay[ver_offset+2], pay[ver_offset+3]])
    } else { 1 };
    let new_id_offset = ver_offset + 4;
    let new_id = if new_id_offset + 4 <= pay.len() {
        u32::from_ne_bytes([pay[new_id_offset], pay[new_id_offset+1], pay[new_id_offset+2], pay[new_id_offset+3]])
    } else { 0 };
    (iface, version, new_id)
}

/// Find the offset of new_id within a bind payload (relative to start of payload).
fn find_new_id_offset_in_bind(pay: &[u8]) -> Option<usize> {
    if pay.len() < 12 { return None; }
    let slen = u32::from_ne_bytes([pay[4], pay[5], pay[6], pay[7]]) as usize;
    let str_padded = (slen + 3) & !3;
    let new_id_offset = 8 + str_padded + 4;
    if new_id_offset + 4 <= pay.len() { Some(new_id_offset) } else { None }
}

/// Find the offset of version within a bind payload (relative to start of payload).
fn find_version_offset_in_bind(pay: &[u8]) -> Option<usize> {
    if pay.len() < 12 { return None; }
    let slen = u32::from_ne_bytes([pay[4], pay[5], pay[6], pay[7]]) as usize;
    let str_padded = (slen + 3) & !3;
    let ver_offset = 8 + str_padded;
    if ver_offset + 4 <= pay.len() { Some(ver_offset) } else { None }
}

fn handle_bind(s: &mut Session, msg: &RawMsg) -> Result<()> {
    let pay = &msg.raw[8..];
    if pay.len() < 12 {
        warn!("bind payload too short");
        send_raw(&s.to_comp, msg)?;
        return Ok(());
    }

    let global_name = u32::from_ne_bytes([pay[0], pay[1], pay[2], pay[3]]);

    // Wayland wire format for wl_registry.bind:
    //   name(u32) | interface_string(4+len+padded) | version(u32) | new_id(u32)
    // The interface string has a 4-byte length header followed by the
    // string + nul + padding to next multiple of 4.
    let str_len_raw = u32::from_ne_bytes([pay[4], pay[5], pay[6], pay[7]]);
    let str_len = str_len_raw as usize;
    let str_padded = (str_len + 3) & !3;
    let str_offset = 8; // 4(name) + 4(str_len)
    let ver_offset = str_offset + str_padded;
    let new_id_offset = ver_offset + 4;

    let new_id = if new_id_offset + 4 <= pay.len() {
        u32::from_ne_bytes([pay[new_id_offset], pay[new_id_offset+1], pay[new_id_offset+2], pay[new_id_offset+3]])
    } else {
        warn!("  bind payload too short for new_id (needs {} bytes, has {})", new_id_offset + 4, pay.len());
        u32::from_ne_bytes([pay[4], pay[5], pay[6], pay[7]])
    };

    // Extract the interface string from the wire message so we can check it
    // before looking up the global (needed for intercepted globals).
    let iface_from_wire: Option<String> = {
        let slen = str_len_raw as usize;
        if slen > 0 && slen < 256 && 8 + slen <= pay.len() {
            let s = String::from_utf8_lossy(&pay[8..8 + slen]).trim_end_matches('\0').to_string();
            Some(s)
        } else {
            None
        }
    };

    // Always intercept binds to our fake injected global (name=1000).
    if global_name == FAKE_GLOBAL_LAYER_SHELL {
        info!("  🎣 intercepted fake bind for zwlr_layer_shell_v1 → cli_oid={new_id}");
        let already_tracked = s.fake_objects.iter().any(|f| f.cli_oid == new_id);
        if !already_tracked {
            s.fake_objects.push(FakeObject {
                cli_oid: new_id,
                iface: "zwlr_layer_shell_v1".into(),
                next_sub_oid: new_id + 1,
                data: String::new(),
            });
        }
        return Ok(());
    }

    // zxdg_output_manager_v1: always forward to Mutter

    let global = s.comp_globals.iter().find(|g| g.name == global_name);
    let iface = global.map(|g| g.interface.as_str()).unwrap_or("?");
    let comp_version = global.map(|g| g.version).unwrap_or(1);
    let cli_version = if ver_offset + 4 <= pay.len() {
        u32::from_ne_bytes([pay[ver_offset], pay[ver_offset+1], pay[ver_offset+2], pay[ver_offset+3]])
    } else {
        info!("  bind payload only {} bytes, no version field — setting default v1", pay.len());
        1
    };
    let msg_size = msg.raw.len();
    info!("bind: name={global_name}, new_id={new_id}, iface={iface}, cli_v={cli_version}, comp_v={comp_version}, msg_size={msg_size}, raw={:02x?}", &msg.raw[..msg_size.min(56)]);

    if iface == "xdg_wm_base" {
        s.cli_xdg_wm_base_id = new_id;
        info!("  identified xdg_wm_base at cli_oid={new_id}");
        // Intercept xdg_wm_base on tertiary registries and forward via primary.
        if s.cli_reg_id > 10 {
            info!("  intercepting xdg_wm_base on tertiary registry -> forwarding via primary oid={}", s.cli_xdg_wm_base_id);
            // Create fake object to forward requests to primary xdg_wm_base
            if !s.fake_objects.iter().any(|f| f.cli_oid == new_id) && new_id > 0 {
                let primary_oid = s.cli_xdg_wm_base_id;
                s.fake_objects.push(FakeObject {
                    cli_oid: new_id,
                    iface: "xdg_wm_base".into(),
                    next_sub_oid: new_id + 1,
                    data: primary_oid.to_string(),
                });
            }
            return Ok(());
        }
    }

    // For PRIMARY registry binds, always allow (global will arrive eventually).
    // Only block non-primary binds to unknown globals.
    if msg.object_id() == s.cli_reg_id {
        // Primary registry — proceed with version clamping
    } else if global_name != FAKE_GLOBAL_LAYER_SHELL && !s.comp_globals.iter().any(|g| g.name == global_name) {
        warn!("  ⚠ intercepted bind to unknown global name={}, iface='{}' — preventing compositor error", global_name, iface);
        return Ok(());
    }

    // Clamp version: if the client requests a version higher than what the
    // compositor supports, Mutter rejects the bind with "invalid arguments".
    // We must cap cli_version to comp_version before forwarding.
    // The version is at offset 8+str_padded (after name(4) + str_len(4) + string(str_padded)).
    if cli_version > comp_version {
        info!("  clamping bind version {} -> {} for {} (compositor only supports up to v{})",
            cli_version, comp_version, iface, comp_version);
        let mut clamped = msg.raw.clone();
        let raw_ver_offset = 8 + 8 + str_padded; // msg.raw[8+8+str_padded]
        clamped[raw_ver_offset..raw_ver_offset+4].copy_from_slice(&comp_version.to_ne_bytes());
        return send_raw_raw(&s.to_comp, clamped, &msg.fds);
    }

    // Forward to compositor
    send_raw(&s.to_comp, msg)
}

/// Handle a message targeting a fake bridge-managed object.
fn handle_fake_msg(s: &mut Session, fake: &FakeObject, msg: &RawMsg) -> Result<()> {
    match fake.iface.as_str() {
        "zwlr_layer_shell_v1" => handle_layer_shell_request(s, fake, msg)?,
        "zwlr_layer_surface_v1" => handle_layer_surface_request(s, fake, msg)?,
        "xdg_surface" => handle_xdg_surface_request(s, fake, msg)?,
        "zxdg_output_manager_v1" => handle_xdg_output_manager_request(s, fake, msg)?,
        "zxdg_output_v1" => handle_xdg_output_request(s, fake, msg)?,
        "xdg_wm_base" => handle_xdg_wm_base_forward(s, fake, msg)?,
        other => {
            debug!("  transparent/silent iface '{other}', oid={}", fake.cli_oid);
        }
    }
    Ok(())
}

/// Forward xdg_wm_base requests from a remapped xdg_wm_base to the real compositor OID.
fn handle_xdg_wm_base_forward(s: &mut Session, fake: &FakeObject, msg: &RawMsg) -> Result<()> {
    if let Ok(real_oid) = fake.data.parse::<u32>() {
        let mut rewritten = msg.raw.clone();
        rewritten[0..4].copy_from_slice(&real_oid.to_ne_bytes());
        return send_raw_raw(&s.to_comp, rewritten, &msg.fds);
    }
    send_raw(&s.to_comp, msg)
}

// ─── xdg-output handler ────────────────────────────────────────────────────

/// Handle requests on the fake zxdg_output_manager_v1 object.
/// Protocol:
///   op 0: destroy()
///   op 1: get_xdg_output(new_id, output)
fn handle_xdg_output_manager_request(s: &mut Session, fake: &FakeObject, msg: &RawMsg) -> Result<()> {
    let op = msg.opcode();
    match op {
        0 => {
            // destroy — noop, just forget the object
            info!("  zxdg_output_manager_v1.destroy");
        }
        1 => {
            // get_xdg_output(new_id, output)
            let pay = &msg.raw[8..];
            if pay.len() < 8 {
                info!("  get_xdg_output payload too short");
                return Ok(());
            }
            let xdg_out_new_id = u32::from_ne_bytes([pay[0], pay[1], pay[2], pay[3]]);
            let output_oid = u32::from_ne_bytes([pay[4], pay[5], pay[6], pay[7]]);
            info!("  Intercepted get_xdg_output: new_id={xdg_out_new_id}, output_oid={output_oid}");

            // Track this xdg_output as a fake object
            s.fake_objects.push(FakeObject {
                cli_oid: xdg_out_new_id,
                iface: "zxdg_output_v1".into(),
                next_sub_oid: xdg_out_new_id + 1,
                data: String::new(),
            });

            // Now send synthetic xdg_output events to the client so it doesn't hang.
            // Logical position from our monitor tracking:
            let (lx, ly) = s.monitors.iter()
                .find(|m| m.comp_oid == output_oid)
                .map(|m| (m.x, m.y))
                .unwrap_or((0, 0));

            // Send logical_position(x, y) — op 2
            let mut pos_pay = Vec::with_capacity(8);
            pos_pay.extend_from_slice(&lx.to_ne_bytes());
            pos_pay.extend_from_slice(&ly.to_ne_bytes());
            send_raw(&s.to_cli, &RawMsg {
                raw: make_raw(xdg_out_new_id, 2, &pos_pay),
                fds: Vec::new(),
            })?;

            // Send output_name — op 4
            let output_name_str = s.monitors.iter()
                .find(|m| m.comp_oid == output_oid)
                .and_then(|m| m.name.clone())
                .unwrap_or_else(|| format!("wayland-bridge-{}", output_oid));
            let mut name = output_name_str;
            name.push('\0');
            let name_slen = name.len() as u32;
            let name_padded = ((name_slen as usize) + 3) & !3;
            let mut name_pay = Vec::with_capacity(4 + name_padded);
            name_pay.extend_from_slice(&name_slen.to_ne_bytes());
            name_pay.extend_from_slice(name.as_bytes());
            while name_pay.len() < (4 + name_padded) {
                name_pay.push(0);
            }
            send_raw(&s.to_cli, &RawMsg {
                raw: make_raw(xdg_out_new_id, 4, &name_pay),
                fds: Vec::new(),
            })?;

            // Send done(serial) — op 1 (required to complete the xdg_output round-trip)
            let done_pay = 1u32.to_ne_bytes(); // serial=1
            send_raw(&s.to_cli, &RawMsg {
                raw: make_raw(xdg_out_new_id, 1, &done_pay),
                fds: Vec::new(),
            })?;

            info!("  Injected synthetic xdg_output events: logical_position({lx},{ly}), output_name='wayland-2-gnome'");
        }
        _ => {
            info!("  unknown zxdg_output_manager_v1 op={}", op);
        }
    }
    Ok(())
}

/// Handle requests on a fake zxdg_output_v1 object.
/// Protocol:
///   op 0: destroy()
fn handle_xdg_output_request(s: &mut Session, fake: &FakeObject, msg: &RawMsg) -> Result<()> {
    let op = msg.opcode();
    match op {
        0 => {
            // destroy — noop
            info!("  zxdg_output_v1.destroy oid={}", fake.cli_oid);
        }
        _ => {
            info!("  unknown zxdg_output_v1 op={}", op);
        }
    }
    Ok(())
}

// ─── Layer shell protocol handlers ──────────────────────────────────────────

fn handle_layer_shell_request(s: &mut Session, fake: &FakeObject, msg: &RawMsg) -> Result<()> {
    let op = msg.opcode();
    match op {
        0 => {
            // get_layer_surface(new_id, surface, output, layer, namespace)
            let pay = &msg.raw[8..];
            if pay.len() < 24 {
                return Ok(());
            }

            let cli_layer_surf_id = u32::from_ne_bytes([pay[0], pay[1], pay[2], pay[3]]);
            let surface_id = u32::from_ne_bytes([pay[4], pay[5], pay[6], pay[7]]);
            let output_id = u32::from_ne_bytes([pay[8], pay[9], pay[10], pay[11]]);
            let layer = u32::from_ne_bytes([pay[12], pay[13], pay[14], pay[15]]);

            info!(
                "  Intercepted get_layer_surface: cli_oid={}, surface={}, output={}, layer={}",
                cli_layer_surf_id, surface_id, output_id, layer
            );

            if s.cli_xdg_wm_base_id == 0 {
                error!("  Cannot translate: client hasn't bound xdg_wm_base yet!");
                return Ok(());
            }

            // 1. Create xdg_surface on compositor
            let comp_xdg_surf_id = s.next_oid;
            s.next_oid += 1;
            let mut xdg_surf_pay = Vec::new();
            xdg_surf_pay.extend_from_slice(&comp_xdg_surf_id.to_ne_bytes());
            xdg_surf_pay.extend_from_slice(&surface_id.to_ne_bytes());
            
            send_raw(&s.to_comp, &RawMsg {
                raw: make_raw(s.cli_xdg_wm_base_id, 2, &xdg_surf_pay), // op 2 = get_xdg_surface
                fds: Vec::new(),
            })?;

            // 2. Create xdg_toplevel from that xdg_surface
            let comp_toplevel_id = s.next_oid;
            s.next_oid += 1;
            let mut toplevel_pay = Vec::new();
            toplevel_pay.extend_from_slice(&comp_toplevel_id.to_ne_bytes());

            send_raw(&s.to_comp, &RawMsg {
                raw: make_raw(comp_xdg_surf_id, 1, &toplevel_pay), // op 1 = get_toplevel
                fds: Vec::new(),
            })?;

            // 3. Set toplevel properties to mimic a layer
            // Handle decorations (remove borders in GNOME)
            if s.comp_dec_mgr_name != 0 {
                if s.comp_dec_mgr_id == 0 {
                    s.comp_dec_mgr_id = s.next_oid;
                    s.next_oid += 1;
                    // Bind decoration manager: name, interface_slen, interface, version, new_id
                    // Note: Registry bind is op 0: name(u32), iface(string), ver(u32), new_id(u32)
                    let mut bind_pay = Vec::new();
                    bind_pay.extend_from_slice(&s.comp_dec_mgr_name.to_ne_bytes());
                    let iface = "zxdg_decoration_manager_v1\0";
                    let slen = iface.len() as u32;
                    bind_pay.extend_from_slice(&slen.to_ne_bytes());
                    bind_pay.extend_from_slice(iface.as_bytes());
                    while bind_pay.len() % 4 != 0 { bind_pay.push(0); }
                    bind_pay.extend_from_slice(&1u32.to_ne_bytes()); // version 1
                    bind_pay.extend_from_slice(&s.comp_dec_mgr_id.to_ne_bytes());
                    
                    send_raw(&s.to_comp, &RawMsg {
                        raw: make_raw(s.comp_reg_id, 0, &bind_pay),
                        fds: Vec::new(),
                    })?;
                    info!("  Bound zxdg_decoration_manager_v1 internally");
                }

                let decoration_id = s.next_oid;
                s.next_oid += 1;
                let mut dec_pay = Vec::new();
                dec_pay.extend_from_slice(&decoration_id.to_ne_bytes());
                dec_pay.extend_from_slice(&comp_toplevel_id.to_ne_bytes());
                
                // get_toplevel_decoration(new_id, toplevel)
                send_raw(&s.to_comp, &RawMsg {
                    raw: make_raw(s.comp_dec_mgr_id, 1, &dec_pay),
                    fds: Vec::new(),
                })?;
                
                // set_mode(1) where 1 = Client-side decorations
                let mut mode_pay = Vec::new();
                mode_pay.extend_from_slice(&1u32.to_ne_bytes());
                send_raw(&s.to_comp, &RawMsg {
                    raw: make_raw(decoration_id, 1, &mode_pay),
                    fds: Vec::new(),
                })?;
                info!("  Disabled server-side decorations for toplevel {}", comp_toplevel_id);
            }

            // 4. Input Passthrough (Click-through) by default for Background/Bottom layers
            if layer == 0 || layer == 1 { // Background or Bottom
                if s.comp_compositor_name != 0 {
                    if s.comp_compositor_id == 0 {
                        s.comp_compositor_id = s.next_oid;
                        s.next_oid += 1;
                        let mut b_pay = Vec::new();
                        b_pay.extend_from_slice(&s.comp_compositor_name.to_ne_bytes());
                        let iface = "wl_compositor\0";
                        b_pay.extend_from_slice(&(iface.len() as u32).to_ne_bytes());
                        b_pay.extend_from_slice(iface.as_bytes());
                        while b_pay.len() % 4 != 0 { b_pay.push(0); }
                        b_pay.extend_from_slice(&4u32.to_ne_bytes()); // version 4
                        b_pay.extend_from_slice(&s.comp_compositor_id.to_ne_bytes());
                        send_raw(&s.to_comp, &RawMsg { raw: make_raw(s.comp_reg_id, 0, &b_pay), fds: Vec::new() })?;
                    }

                    // Create an empty region to make the surface click-through
                    let region_id = s.next_oid;
                    s.next_oid += 1;
                    let mut r_pay = Vec::new();
                    r_pay.extend_from_slice(&region_id.to_ne_bytes());
                    // wl_compositor.create_region(new_id)
                    send_raw(&s.to_comp, &RawMsg { raw: make_raw(s.comp_compositor_id, 0, &r_pay), fds: Vec::new() })?;

                    // Apply empty region to surface: wl_surface.set_input_region(region)
                    let mut sir_pay = Vec::new();
                    sir_pay.extend_from_slice(&region_id.to_ne_bytes());
                    send_raw(&s.to_comp, &RawMsg {
                        raw: make_raw(surface_id, 3, &sir_pay), // wl_surface.set_input_region is op 3
                        fds: Vec::new(),
                    })?;
                    
                    // Cleanup: wl_region.destroy
                    send_raw(&s.to_comp, &RawMsg {
                        raw: make_raw(region_id, 0, &[]), // wl_region.destroy is op 0
                        fds: Vec::new(),
                    })?;
                    info!("  Set click-through (empty input region) for surface {}", surface_id);
                }
            }

            // Set title
            let title = "Wayland-Gnome-Bridge Overlay\0";
            let mut title_pay = Vec::new();
            let title_len = title.len() as u32;
            title_pay.extend_from_slice(&title_len.to_ne_bytes());
            title_pay.extend_from_slice(title.as_bytes());
            while title_pay.len() % 4 != 0 { title_pay.push(0); }
            send_raw(&s.to_comp, &RawMsg {
                raw: make_raw(comp_toplevel_id, 2, &title_pay), // op 2 = set_title
                fds: Vec::new(),
            })?;

            // 4. Map the IDs for future event translation
            s.translation_map.push(TranslationEntry {
                cli_layer_surface_oid: cli_layer_surf_id,
                cli_xdg_surface_oid: 0,
                comp_xdg_surface_oid: comp_xdg_surf_id,
                comp_toplevel_oid: comp_toplevel_id,
                cli_surface_oid: surface_id,
                requested_output_oid: output_id,
                pending_width: 0,
                pending_height: 0,
            });

            // Set App ID so GNOME Shell can identify it
            let app_id = "wayland-2-gnome\0";
            let mut app_id_pay = Vec::new();
            let app_id_len = app_id.len() as u32;
            app_id_pay.extend_from_slice(&app_id_len.to_ne_bytes());
            app_id_pay.extend_from_slice(app_id.as_bytes());
            while app_id_pay.len() % 4 != 0 { app_id_pay.push(0); }
            send_raw(&s.to_comp, &RawMsg {
                raw: make_raw(comp_toplevel_id, 3, &app_id_pay), // op 3 = set_app_id
                fds: Vec::new(),
            })?;

            // 5. Track as a fake object to intercept its requests (like ack_configure)
            s.fake_objects.push(FakeObject {
                cli_oid: cli_layer_surf_id,
                iface: "zwlr_layer_surface_v1".into(),
                next_sub_oid: 0,
                data: String::new(),
            });

            info!("  Translated LayerSurface {} to XDG Toplevel {}", cli_layer_surf_id, comp_toplevel_id);
        }
        1 => {
            // destroy
            info!("  layer_shell.destroy oid={}", fake.cli_oid);
        }
        other => {
            debug!("  layer_shell: unknown op={other}, oid={}", fake.cli_oid);
        }
    }
    Ok(())
}

fn handle_xdg_surface_request(s: &mut Session, fake: &FakeObject, msg: &RawMsg) -> Result<()> {
    let op = msg.opcode();
    let pay = &msg.raw[8..];
    
    let entry = s.translation_map.iter().find(|e| e.cli_xdg_surface_oid == fake.cli_oid).cloned();
    let entry = match entry {
        Some(e) => e,
        None => return Ok(()),
    };

    match op {
        0 => { // destroy
            info!("  xdg_surface.destroy: alias oid={}", fake.cli_oid);
            s.fake_objects.retain(|f| f.cli_oid != fake.cli_oid);
        }
        2 => { // get_popup(new_id, parent, positioner)
            if pay.len() >= 12 {
                let new_id = u32::from_ne_bytes([pay[0], pay[1], pay[2], pay[3]]);
                let parent_id = u32::from_ne_bytes([pay[4], pay[5], pay[6], pay[7]]);
                let pos_id = u32::from_ne_bytes([pay[8], pay[9], pay[10], pay[11]]);
                
                info!("  Intercepted xdg_surface.get_popup: new_id={}, parent={}, pos={}", new_id, parent_id, pos_id);
                
                // We need to forward this to the REAL xdg_surface on the compositor.
                // If parent_id matches any of our managed xdg_surfaces, we must translate it.
                let mut real_parent = parent_id;
                if let Some(p_entry) = s.translation_map.iter().find(|e| e.cli_xdg_surface_oid == parent_id) {
                    real_parent = p_entry.comp_xdg_surface_oid;
                }

                let mut new_pay = Vec::new();
                new_pay.extend_from_slice(&new_id.to_ne_bytes());
                new_pay.extend_from_slice(&real_parent.to_ne_bytes());
                new_pay.extend_from_slice(&pos_id.to_ne_bytes());
                
                send_raw(&s.to_comp, &RawMsg {
                    raw: make_raw(entry.comp_xdg_surface_oid, 2, &new_pay),
                    fds: Vec::new(),
                })?;
            }
        }
        4 => { // ack_configure(serial)
            send_raw(&s.to_comp, &RawMsg {
                raw: make_raw(entry.comp_xdg_surface_oid, 4, pay),
                fds: Vec::new(),
            })?;
        }
        _ => {
            debug!("  xdg_surface (alias): ignored op={}", op);
        }
    }
    Ok(())
}

fn handle_layer_surface_request(s: &mut Session, fake: &FakeObject, msg: &RawMsg) -> Result<()> {
    let op = msg.opcode();
    let pay = &msg.raw[8..];
    
    let entry = s.translation_map.iter().find(|e| e.cli_layer_surface_oid == fake.cli_oid).cloned();

    match op {
        0 => {
            // set_size(width, height)
            if let (Some(e), true) = (entry, pay.len() >= 8) {
                let w = u32::from_ne_bytes([pay[0], pay[1], pay[2], pay[3]]);
                let h = u32::from_ne_bytes([pay[4], pay[5], pay[6], pay[7]]);
                info!("  Translating layer_surface.set_size({w}, {h}) to XDG constraints");
                
                // En XDG Shell, no hay set_size directo, usamos set_min_size y set_max_size
                // para forzar al compositor a darnos exactamente ese tamaño.
                let mut size_pay = Vec::new();
                size_pay.extend_from_slice(&w.to_ne_bytes());
                size_pay.extend_from_slice(&h.to_ne_bytes());
                
                send_raw(&s.to_comp, &RawMsg {
                    raw: make_raw(e.comp_toplevel_oid, 8, &size_pay), // op 8 = set_min_size
                    fds: Vec::new(),
                })?;
                send_raw(&s.to_comp, &RawMsg {
                    raw: make_raw(e.comp_toplevel_oid, 7, &size_pay), // op 7 = set_max_size
                    fds: Vec::new(),
                })?;
            }
        }
        1 => {
            // set_anchor(anchor)
            if let (Some(e), true) = (entry, pay.len() >= 4) {
                let anchor = u32::from_ne_bytes([pay[0], pay[1], pay[2], pay[3]]);
                // If anchor covers opposite edges, likely fullscreen request
                if anchor == 15 { // top | bottom | left | right
                    info!("  Layer anchor is Fullscreen -> set_fullscreen on output {}", e.requested_output_oid);
                    
                    let mut fs_pay = Vec::new();
                    fs_pay.extend_from_slice(&e.requested_output_oid.to_ne_bytes());
                    
                    send_raw(&s.to_comp, &RawMsg {
                        raw: make_raw(e.comp_toplevel_oid, 11, &fs_pay), // op 11 = set_fullscreen(output)
                        fds: Vec::new(),
                    })?;
                }
            }
        }
        2 => {
            // set_exclusive_zone(zone)
            if let (Some(_e), true) = (entry, pay.len() >= 4) {
                let zone = i32::from_ne_bytes([pay[0], pay[1], pay[2], pay[3]]);
                info!("  Layer exclusive zone: {} (Note: Not fully supported in GNOME without extensions)", zone);
            }
        }
        3 => {
            // set_margin(top, right, bottom, left)
            if let (Some(_e), true) = (entry, pay.len() >= 16) {
                let t = i32::from_ne_bytes([pay[0], pay[1], pay[2], pay[3]]);
                let r = i32::from_ne_bytes([pay[4], pay[5], pay[6], pay[7]]);
                let b = i32::from_ne_bytes([pay[8], pay[9], pay[10], pay[11]]);
                let l = i32::from_ne_bytes([pay[12], pay[13], pay[14], pay[15]]);
                info!("  Layer margin: t={} r={} b={} l={}", t, r, b, l);
            }
        }
        4 => {
            // set_keyboard_interactivity(interactive)
            if let (Some(_e), true) = (entry, pay.len() >= 4) {
                let interactive = u32::from_ne_bytes([pay[0], pay[1], pay[2], pay[3]]);
                info!("  Layer interactivity: {} -> Adjusting XDG Toplevel", interactive);
            }
        }
        5 => {
            // get_popup(popup)
            info!("  Layer get_popup (op 5) ignored on layer_surface {}", fake.cli_oid);
        }
        6 => {
            // ack_configure(serial)
            if let (Some(e), true) = (entry, pay.len() >= 4) {
                let serial = u32::from_ne_bytes([pay[0], pay[1], pay[2], pay[3]]);
                info!("  layer_surface.ack_configure: oid={}, serial={} -> forwarding to xdg_surface {}", fake.cli_oid, serial, e.comp_xdg_surface_oid);
                
                // Forward ack_configure to the real xdg_surface
                send_raw(&s.to_comp, &RawMsg {
                    raw: make_raw(e.comp_xdg_surface_oid, 4, pay), // xdg_surface.ack_configure is op 4
                    fds: Vec::new(),
                })?;
            }
        }
        7 => {
            // destroy
            info!("  layer_surface.destroy: oid={}", fake.cli_oid);
            if let Some(e) = entry {
                // Destroy toplevel
                send_raw(&s.to_comp, &RawMsg {
                    raw: make_raw(e.comp_toplevel_oid, 0, &[]),
                    fds: Vec::new(),
                })?;
                // Destroy xdg_surface
                send_raw(&s.to_comp, &RawMsg {
                    raw: make_raw(e.comp_xdg_surface_oid, 0, &[]),
                    fds: Vec::new(),
                })?;
                s.translation_map.retain(|te| te.cli_layer_surface_oid != fake.cli_oid);
            }
            s.fake_objects.retain(|f| f.cli_oid != fake.cli_oid);
        }
        _ => {
            // Other ops like set_size, set_anchor... 
            // For now, we just log them. In a complete version, 
            // we'd map these to xdg_toplevel.set_fullscreen, etc.
            debug!("  layer_surface: op={op} ignored for now");
        }
    }
    Ok(())
}

// ─── Compositor messages ────────────────────────────────────────────────────

fn handle_comp(s: &mut Session, msg: &RawMsg) -> Result<()> {
    let oid = msg.object_id();
    let op = msg.opcode();

    if s.tracked_outputs.iter().any(|t| t.comp_oid == oid) {
        sniff_output_event(s, oid, op, &msg.raw[8..]);
    }

    if s.globals_collected && oid == s.comp_reg_id && op == 0 {
        let pay = &msg.raw[8..];
        if pay.len() >= 12 {
            let gname = u32::from_ne_bytes([pay[0], pay[1], pay[2], pay[3]]);
            let slen_raw = u32::from_ne_bytes([pay[4], pay[5], pay[6], pay[7]]);
            let slen = slen_raw as usize;
            let iface_end = (8 + slen).min(pay.len());
            let iface = if iface_end > 8 {
                let mut si = String::new();
                for &b in &pay[8..iface_end] { if b == 0 { break; } si.push(b as char); }
                si
            } else { String::new() };
            let ver_offset = 8 + ((slen + 3) & !3);
            let version = if ver_offset + 4 <= pay.len() {
                u32::from_ne_bytes([pay[ver_offset], pay[ver_offset+1], pay[ver_offset+2], pay[ver_offset+3]])
            } else { 1 };
            info!("  Dynamic global: name={}, iface='{}', v{}", gname, iface, version);
            if iface == "wl_output" {
                handle_dynamic_output_global(s, gname, &iface, version)?;
            }
            if !s.comp_globals.iter().any(|g| g.name == gname) {
                s.comp_globals.push(GlobalInfo { name: gname, interface: iface, version });
            }
            return send_raw(&s.to_cli, msg);
        }
    }

    if let Some(pos) = s.translation_map.iter().position(|e| e.comp_toplevel_oid == oid) {
        if op == 0 {
            let pay = &msg.raw[8..];
            if pay.len() >= 8 {
                let w = u32::from_ne_bytes([pay[0], pay[1], pay[2], pay[3]]);
                let h = u32::from_ne_bytes([pay[4], pay[5], pay[6], pay[7]]);
                s.translation_map[pos].pending_width = w;
                s.translation_map[pos].pending_height = h;
                debug!("  xdg_toplevel.configure: stored size {}x{}", w, h);
                return Ok(());
            }
        }
    }

    if let Some(entry) = s.translation_map.iter().find(|e| e.comp_xdg_surface_oid == oid) {
        if op == 0 {
            let pay = &msg.raw[8..];
            let serial = u32::from_ne_bytes([pay[0], pay[1], pay[2], pay[3]]);
            info!("  Translating xdg_surface.configure(serial={}) -> layer_surface.configure({}x{})",
                serial, entry.pending_width, entry.pending_height);
            let mut l_pay = Vec::new();
            l_pay.extend_from_slice(&serial.to_ne_bytes());
            l_pay.extend_from_slice(&entry.pending_width.to_ne_bytes());
            l_pay.extend_from_slice(&entry.pending_height.to_ne_bytes());
            return send_raw(&s.to_cli, &RawMsg {
                raw: make_raw(entry.cli_layer_surface_oid, 0, &l_pay),
                fds: Vec::new(),
            });
        }
    }

    if oid == 1 && op == 1 {
        let pay = &msg.raw[8..];
        if pay.len() >= 4 {
            let deleted_id = u32::from_ne_bytes([pay[0], pay[1], pay[2], pay[3]]);
            if let Some(pos) = s.translation_map.iter().position(|e| e.cli_layer_surface_oid == deleted_id) {
                info!("  Cleaning up translation for deleted OID {}", deleted_id);
                s.translation_map.remove(pos);
            }
        }
    }


    debug!("comp -> cli: oid={}, op={}, size={} raw={:02x?}", oid, msg.opcode(), msg.raw.len(), &msg.raw[..msg.raw.len().min(32)]);

    if oid == 1 && msg.opcode() == 0 {
        let pay = &msg.raw[8..];
        let err_oid = if pay.len() >= 4 { u32::from_ne_bytes([pay[0], pay[1], pay[2], pay[3]]) } else { 0 };
        let err_code = if pay.len() >= 8 { u32::from_ne_bytes([pay[4], pay[5], pay[6], pay[7]]) } else { 0 };
        let err_msg = if pay.len() > 8 { String::from_utf8_lossy(&pay[8..]).trim_end_matches('\0').to_string() } else { String::new() };
        error!("COMPOSITOR PROTOCOL ERROR: object={}, code={}, msg='{}'", err_oid, err_code, err_msg);
        debug!("  error raw ({} bytes): {:02x?}", msg.raw.len(), &msg.raw);
        // Also log the string bytes to check for truncation
        if pay.len() > 8 {
            debug!("  error string bytes: {:02x?} (pay offset 8+, {} bytes available)", &pay[8..], pay.len() - 8);
        }
        return send_raw(&s.to_cli, msg);
    }

    send_raw(&s.to_cli, msg)
}


