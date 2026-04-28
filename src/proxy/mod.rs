//! Phase 4: Selective Byte-Level Proxy
//!
//! Acts as a Unix socket proxy between client and Hyprland.
//! Intercepts ONLY `zwlr_layer_shell_v1` protocol messages and translates them.
//! Everything else (EGL, wl_drm, wl_surface, Vulkan, etc.) passes through untouched.

use std::collections::{HashMap, VecDeque};
use std::os::unix::io::{AsRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use log::{error, info, warn};
use nix::sys::socket::{recvmsg, sendmsg, ControlMessageOwned, MsgFlags};
use nix::cmsg_space;

use crate::BridgeConfig;

// ── Wayland wire protocol constants ─────────────────────────────────────────

#[repr(C, packed)]
struct WlMsgHeader {
    object_id: u32,
    opcode_size: u32, // upper 16 = size, lower 16 = opcode
}

impl WlMsgHeader {
    fn opcode(&self) -> u16 { (self.opcode_size & 0xffff) as u16 }
    fn size(&self) -> u16 { (self.opcode_size >> 16) as u16 }
}

// Standard object IDs
const WL_DISPLAY_ID: u32 = 1;
const WL_DISPLAY_ERROR: u16 = 0;
const WL_DISPLAY_GET_REGISTRY: u16 = 1;
const WL_REGISTRY_GLOBAL: u16 = 0;
const WL_REGISTRY_GLOBAL_REMOVE: u16 = 1;
const WL_REGISTRY_BIND: u16 = 0;

// ── Proxy state per client connection ──────────────────────────────────────

struct ClientProxy {
    to_client: UnixStream,
    to_compositor: UnixStream,
    running: Arc<AtomicBool>,
}

impl ClientProxy {
    fn new(to_client: UnixStream, to_compositor: UnixStream, running: Arc<AtomicBool>) -> Self {
        Self { to_client, to_compositor, running }
    }

    /// Read one raw Wayland message from a stream, returning (header, raw_data, fds).
    fn read_msg(stream: &UnixStream) -> Result<Option<(WlMsgHeader, Vec<u8>, Vec<OwnedFd>)>> {
        let mut header_buf = [0u8; 8];
        let mut n = 0;
        while n < 8 {
            match stream.read_at_nonblocking(&mut header_buf[n..])? {
                0 if n == 0 => return Ok(None), // EOF / would block
                0 => bail!("Short header read"),
                Some(s) => n += s,
                None => { /* would block */ return Ok(None); }
            }
        }
        let header: &WlMsgHeader = unsafe { &*(header_buf.as_ptr() as *const WlMsgHeader) };
        let total_size = header.size() as usize;
        if total_size < 8 {
            bail!("Message size too small: {}", total_size);
        }
        let payload_size = total_size - 8;
        let mut payload = vec![0u8; payload_size];
        n = 0;
        while n < payload_size {
            match stream.read_at_nonblocking(&mut payload[n..])? {
                0 => bail!("Short payload read"),
                Some(s) => n += s,
                None => bail!("Would block mid-payload"),
            }
        }

        // Read any attached FDs via SCM_RIGHTS
        let fds = recv_fds(stream.as_raw_fd())?;

        Ok(Some((*header, payload, fds)))
    }

    fn write_raw(stream: &UnixStream, header: &WlMsgHeader, payload: &[u8], fds: &[OwnedFd]) -> Result<()> {
        let hdr_bytes = unsafe {
            std::slice::from_raw_parts(
                header as *const WlMsgHeader as *const u8,
                std::mem::size_of::<WlMsgHeader>(),
            )
        };
        let mut iov = [
            libc::iovec { iov_base: hdr_bytes.as_ptr() as *mut libc::c_void, iov_len: hdr_bytes.len() },
            libc::iovec { iov_base: payload.as_ptr() as *mut libc::c_void, iov_len: payload.len() },
        ];

        let mut cmsg_buf = vec![0u8; libc::CMSG_SPACE((fds.len() * std::mem::size_of::<RawFd>()) as _) as _];
        let mut cmsg = unsafe {
            libc::msghdr {
                msg_name: std::ptr::null_mut(),
                msg_namelen: 0,
                msg_iov: iov.as_mut_ptr(),
                msg_iovlen: iov.len() as _,
                msg_control: cmsg_buf.as_mut_ptr() as *mut _,
                msg_controllen: cmsg_buf.len() as _,
                msg_flags: 0,
            }
        };

        if !fds.is_empty() {
            let fd_raws: Vec<RawFd> = fds.iter().map(|f| f.as_raw_fd()).collect();
            let cmsg_hdr = unsafe {
                libc::CMSG_FIRSTHDR(&cmsg)
            };
            if let Some(hdr) = cmsg_hdr.as_mut() {
                hdr.cmsg_level = libc::SOL_SOCKET;
                hdr.cmsg_type = libc::SCM_RIGHTS;
                hdr.cmsg_len = libc::CMSG_LEN((fd_raws.len() * std::mem::size_of::<RawFd>()) as _) as _;
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        fd_raws.as_ptr(),
                        libc::CMSG_DATA(hdr) as *mut RawFd,
                        fd_raws.len(),
                    );
                }
            }
        } else {
            cmsg.msg_control = std::ptr::null_mut();
            cmsg.msg_controllen = 0;
        }

        unsafe {
            if libc::sendmsg(stream.as_raw_fd(), &cmsg, libc::MSG_NOSIGNAL) < 0 {
                bail!("sendmsg failed: {}", std::io::Error::last_os_error());
            }
        }
        Ok(())
    }
}

fn recv_fds(fd: RawFd) -> Result<Vec<OwnedFd>> {
    let mut cmsg_space = cmsg_space!(RawFd, 4);
    let mut buf = [0u8; 1];
    let msg = recvmsg(
        fd,
        &[libc::iovec {
            iov_base: buf.as_mut_ptr() as *mut _,
            iov_len: 0, // peek only
        }],
        Some(&mut cmsg_space),
        MsgFlags::MSG_DONTWAIT | MsgFlags::MSG_PEEK,
    )?;

    let mut fds = Vec::new();
    for cmsg in msg.cmsgs() {
        if let ControlMessageOwned::ScmRights(fds_slice) = cmsg {
            for &raw_fd in fds_slice {
                if raw_fd >= 0 {
                    fds.push(unsafe { OwnedFd::from_raw_fd(raw_fd) });
                }
            }
        }
    }
    Ok(fds)
}

// ── Layer-shell interception state ──────────────────────────────────────────

struct ObjectMap {
    /// Maps server-assigned object IDs to their interpretation.
    /// None = passthrough, Some(name) = intercepted layer-shell.
    objects: Vec<Option<ObjectKind>>,
    /// The wl_registry object ID (assigned when we spoof it).
    registry_id: u32,
    /// Next object ID to assign for intercepted objects.
}

#[derive(Clone, Debug)]
enum ObjectKind {
    Registry,
    LayerShell,
    LayerSurface,
}

// ── Main proxy loop ─────────────────────────────────────────────────────────

pub fn run_proxy_loop(config: Arc<BridgeConfig>) -> Result<()> {
    let socket_path = config.socket_path();
    let compositor_display = &config.compositor_display;

    // Remove old socket
    let _ = std::fs::remove_file(&socket_path);
    let listen_socket = std::os::unix::net::UnixListener::bind(&socket_path)
        .context("Failed to bind listener socket")?;
    // Set permissions
    std::fs::set_permissions(&socket_path, std::os::unix::fs::PermissionsExt::from_mode(0o777))?;
    info!("Listening on {socket_path}, proxying to {compositor_display}");

    for stream in listen_socket.incoming() {
        let client = stream.context("Accept failed")?;
        client.set_nonblocking(true)?;

        // Connect to real compositor
        let compositor_socket = std::path::Path::new("/run/user/1000")
            .join(compositor_display);
        let compositor = UnixStream::connect(&compositor_socket)
            .context("Failed to connect to compositor")?;
        compositor.set_nonblocking(true)?;

        info!("New client connected, proxying to {compositor_display}");
        let running = Arc::new(AtomicBool::new(true));
        let proxy = Arc::new(ClientProxy::new(client, compositor, running.clone()));

        // Run the bidirectional proxy
        if let Err(e) = run_client_loop(proxy.clone()) {
            error!("Client loop error: {e}");
        }
    }

    Ok(())
}

fn run_client_loop(proxy: Arc<ClientProxy>) -> Result<()> {
    let mut object_map = ObjectMap {
        objects: vec![None; 256], // pre-allocate space for up to 256 object IDs
        registry_id: 0,
    };

    // Phase 1: Initial handshake (client sends get_registry)
    // We need to intercept this to inject our zwlr_layer_shell_v1 global.

    loop {
        // Read from client → compositor
        let msg = ClientProxy::read_msg(&proxy.to_client)?;
        if let Some((header, payload, fds)) = msg {
            if header.object_id == WL_DISPLAY_ID && header.opcode() == WL_DISPLAY_GET_REGISTRY {
                // INTERCEPT: forward but track the registry ID
                // The payload of get_registry is the new_id (registry object)
                let registry_id = parse_new_id(&payload)?;
                info!("Client get_registry → id={}", registry_id);
                object_map.registry_id = registry_id;
                object_map.ensure_capacity(registry_id as usize);
                object_map.objects[registry_id as usize] = Some(ObjectKind::Registry);

                // Forward to compositor
                ClientProxy::write_raw(&proxy.to_compositor, &header, &payload, &fds)?;
            } else if object_map.objects.get(header.object_id as usize)
                .and_then(|o| o.as_ref())
                .map_or(true, |k| !matches!(k, ObjectKind::Registry))
            {
                // Not a registry object → passthrough
                ClientProxy::write_raw(&proxy.to_compositor, &header, &payload, &fds)?;
            } else {
                // Registry request → intercept
                intercept_registry_request(&proxy, &mut object_map, &header, &payload, &fds)?;
            }
        }

        // Read from compositor → client
        let msg = ClientProxy::read_msg(&proxy.to_compositor)?;
        if let Some((header, payload, fds)) = msg {
            // Check if this is a compositor event to our intercepted objects
            if object_map.objects.get(header.object_id as usize)
                .and_then(|o| o.as_ref())
                .map_or(false, |k| matches!(k, ObjectKind::Registry))
            {
                // Registry event → we may need to inject zwlr_layer_shell_v1
                intercept_registry_event(&proxy, &mut object_map, &header, &payload, &fds)?;
            } else {
                // Passthrough
                ClientProxy::write_raw(&proxy.to_client, &header, &payload, &fds)?;
            }
        }

        // TODO: handle would-block with select/poll/epoll
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

fn parse_new_id(payload: &[u8]) -> Result<u32> {
    if payload.len() < 4 {
        bail!("Payload too short for new_id");
    }
    let id = u32::from_ne_bytes([payload[0], payload[1], payload[2], payload[3]]);
    Ok(id)
}

fn ensure_capacity(self: &mut ObjectMap, idx: usize) {
    if idx >= self.objects.len() {
        self.objects.resize(idx + 256, None);
    }
}

fn intercept_registry_request(
    proxy: &ClientProxy,
    object_map: &mut ObjectMap,
    header: &WlMsgHeader,
    payload: &[u8],
    fds: &[OwnedFd],
) -> Result<()> {
    let opcode = header.opcode();
    match opcode {
        WL_REGISTRY_BIND => {
            // wl_registry.bind(name, id, interface, version)
            // Parse: name(u32), new_id(u32), interface_string, version(u32)
            if payload.len() < 8 {
                bail!("Short bind payload");
            }
            let _name = u32::from_ne_bytes([payload[0], payload[1], payload[2], payload[3]]);
            let id = u32::from_ne_bytes([payload[4], payload[5], payload[6], payload[7]]);
            // string starts at byte 8
            let interface = parse_string_at(payload, 8)?;

            info!("  bind object_id={}, interface='{}'", id, interface);

            if interface == "zwlr_layer_shell_v1" {
                // Intercept! Track this object
                object_map.ensure_capacity(id as usize);
                object_map.objects[id as usize] = Some(ObjectKind::LayerShell);
                info!("  ↪ Intercepted layer shell v1 (id={})", id);

                // DON'T forward to compositor (we handle it ourselves)
                // But we need to send the registry.bind to compositor with different args?
                // Actually, the compositor doesn't know about zwlr_layer_shell...
                // For now, just swallow it.
                Ok(())
            } else {
                // Forward to compositor
                ClientProxy::write_raw(&proxy.to_compositor, &header, &payload, &fds)
            }
        }
        _ => {
            ClientProxy::write_raw(&proxy.to_compositor, &header, &payload, &fds)
        }
    }
}

fn intercept_registry_event(
    proxy: &ClientProxy,
    object_map: &mut ObjectMap,
    header: &WlMsgHeader,
    payload: &[u8],
    fds: &[OwnedFd],
) -> Result<()> {
    let opcode = header.opcode();
    match opcode {
        WL_REGISTRY_GLOBAL => {
            // wl_registry.global(name, interface, version)
            let _name = u32::from_ne_bytes([payload[0], payload[1], payload[2], payload[3]]);
            let interface = parse_string_at(payload, 4)?;
            info!("  global: '{}'", interface);

            // Forward all globals as-is
            ClientProxy::write_raw(&proxy.to_client, &header, &payload, &fds)?;

            // After each global from compositor, inject our zwlr_layer_shell_v1 global
            if interface == "wl_output" {
                info!("  ↪ Injecting zwlr_layer_shell_v1 after output");
                // Send fake global event: name=999, interface="zwlr_layer_shell_v1", version=4
                inject_fake_global(proxy, object_map, 999u32)?;
            }
        }
        _ => {
            ClientProxy::write_raw(&proxy.to_client, &header, &payload, &fds)?;
        }
    }
    Ok(())
}

fn inject_fake_global(proxy: &ClientProxy, object_map: &mut ObjectMap, name: u32) -> Result<()> {
    let interface = "zwlr_layer_shell_v1\0";
    let version: u32 = 4;

    // Build payload: name(u32) + string_header(u32=len+flags) + string_data + padding + version(u32)
    let str_len = interface.len() as u32; // includes null terminator
    let str_header = str_len; // no flags (MSB clear)

    let padded_len = align4(interface.len());
    let mut payload = Vec::with_capacity(4 + 4 + padded_len + 4);
    payload.extend_from_slice(&name.to_ne_bytes());
    payload.extend_from_slice(&str_header.to_ne_bytes());
    payload.extend_from_slice(interface.as_bytes());
    while payload.len() % 4 != 0 { payload.push(0); }
    payload.extend_from_slice(&version.to_ne_bytes());

    let obj_id = object_map.registry_id;
    let total_size = (8 + payload.len()) as u16;
    let opcode_size = (total_size as u32) << 16 | WL_REGISTRY_GLOBAL as u32;
    let hdr = WlMsgHeader { object_id: obj_id, opcode_size };

    ClientProxy::write_raw(&proxy.to_client, &hdr, &payload, &[])?;
    Ok(())
}

fn parse_string_at(data: &[u8], offset: usize) -> Result<String> {
    if offset + 4 > data.len() {
        bail!("Offset beyond data");
    }
    let len = u32::from_ne_bytes([data[offset], data[offset+1], data[offset+2], data[offset+3]]);
    let has_flag = (len & 0x80000000) != 0;
    let str_len = (len & 0x7fffffff) as usize;
    if str_len == 0 {
        return Ok(String::new());
    }
    let data_start = offset + 4;
    if data_start + str_len - 1 > data.len() {
        bail!("String extends beyond data");
    }
    // String is null-terminated; exclude the null byte
    let str_bytes = &data[data_start..data_start + str_len - 1];
    Ok(String::from_utf8_lossy(str_bytes).to_string())
}

fn align4(n: usize) -> usize {
    (n + 3) & !3
}
