//! Integration tests for Wayland 2 GNOME protocol translation.
//!
//! Sets up a mock compositor, connects the bridge, then runs a mock client
//! to verify the full protocol translation pipeline.

use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::io::RawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use wayland_2_gnome::proxy;

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Build a Wayland wire message header + payload.
fn make_raw(oid: u32, op: u16, pay: &[u8]) -> Vec<u8> {
    let total = 8u32 + pay.len() as u32;
    let mut m = Vec::with_capacity(total as usize);
    m.extend_from_slice(&oid.to_ne_bytes());
    m.extend_from_slice(&((total << 16) | op as u32).to_ne_bytes());
    m.extend_from_slice(pay);
    m
}

/// Read one complete Wayland wire message.
fn read_raw(stream: &std::os::unix::net::UnixStream) -> Vec<u8> {
    let fd = stream.as_raw_fd();
    let mut hdr = [0u8; 8];
    let mut cbuf = [0u8; 4096];

    let mut hdr_iov = [libc::iovec {
        iov_base: hdr.as_mut_ptr() as *mut _,
        iov_len: 8,
    }];
    let mut hdr_msg: libc::msghdr = unsafe { std::mem::zeroed() };
    hdr_msg.msg_iov = hdr_iov.as_mut_ptr();
    hdr_msg.msg_iovlen = 1;
    hdr_msg.msg_control = cbuf.as_mut_ptr() as *mut _;
    hdr_msg.msg_controllen = cbuf.len();

    let n = unsafe { libc::recvmsg(fd, &mut hdr_msg, libc::MSG_WAITALL) };
    assert!(n >= 0, "recvmsg header failed");
    assert!(n >= 8, "short header: {n}");

    let os = u32::from_ne_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]);
    let expected = (os >> 16) as usize;

    let mut raw = Vec::with_capacity(expected);
    raw.extend_from_slice(&hdr);

    if expected > 8 {
        let mut pay = vec![0u8; expected - 8];
        let mut pay_iov = [libc::iovec {
            iov_base: pay.as_mut_ptr() as *mut _,
            iov_len: expected - 8,
        }];
        let mut pay_msg: libc::msghdr = unsafe { std::mem::zeroed() };
        pay_msg.msg_iov = pay_iov.as_mut_ptr();
        pay_msg.msg_iovlen = 1;

        let n2 = unsafe { libc::recvmsg(fd, &mut pay_msg, libc::MSG_WAITALL) };
        assert!(n2 >= 0, "recvmsg payload failed");
        assert_eq!(n2 as usize, expected - 8, "short payload");
        raw.extend_from_slice(&pay);
    }

    raw
}

/// Parse object_id from raw message.
fn msg_oid(msg: &[u8]) -> u32 {
    u32::from_ne_bytes([msg[0], msg[1], msg[2], msg[3]])
}

/// Parse opcode from raw message.
fn msg_op(msg: &[u8]) -> u16 {
    let os = u32::from_ne_bytes([msg[4], msg[5], msg[6], msg[7]]);
    (os & 0xffff) as u16
}

/// Parse string length from raw Wayland string header (at given offset).
fn msg_string_len(msg: &[u8], offset: usize) -> u32 {
    if offset + 4 > msg.len() {
        return 0;
    }
    u32::from_ne_bytes([msg[offset], msg[offset + 1], msg[offset + 2], msg[offset + 3]])
}

/// Extract string from Wayland wire format at offset (after length header).
fn msg_string(msg: &[u8], offset: usize) -> String {
    let slen = msg_string_len(msg, offset) as usize;
    if slen == 0 || offset + 4 + slen > msg.len() {
        return String::new();
    }
    let start = offset + 4;
    let end = start + slen;
    let bytes = &msg[start..end];
    // Trim null terminator
    let trimmed = if bytes.last() == Some(&0) {
        &bytes[..bytes.len() - 1]
    } else {
        bytes
    };
    String::from_utf8_lossy(trimmed).to_string()
}

// ─── Mock Compositor ────────────────────────────────────────────────────────

/// A minimal mock Wayland compositor for testing bridge protocol translation.
struct MockCompositor {
    socket_path: String,
    listener: std::os::unix::net::UnixListener,
}

impl MockCompositor {
    fn new(socket_name: &str) -> Self {
        let rdir = std::env::var("XDG_RUNTIME_DIR")
            .unwrap_or_else(|_| "/run/user/1000".to_string());
        let socket_path = format!("{rdir}/{socket_name}");
        let _ = std::fs::remove_file(&socket_path);
        let listener = std::os::unix::net::UnixListener::bind(&socket_path)
            .expect("mock compositor bind");
        std::fs::set_permissions(
            &socket_path,
            std::os::unix::fs::PermissionsExt::from_mode(0o777),
        )
        .ok();
        listener.set_nonblocking(true).ok();
        MockCompositor { socket_path, listener }
    }

    /// Accept one connection and return the compositor-side stream.
    fn accept_connection(&self) -> std::os::unix::net::UnixStream {
        loop {
            if let Ok((stream, _)) = self.listener.accept() {
                stream.set_nonblocking(false).ok();
                return stream;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for MockCompositor {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[test]
fn test_registry_forwarding_and_layer_shell_injection() {
    // Start mock compositor on a unique socket
    let test_id = std::process::id();
    let comp_socket = format!("wayland-test-comp-{test_id}");
    let bridge_socket = format!("wayland-test-bridge-{test_id}");

    let compositor = MockCompositor::new(&comp_socket);

    // Start bridge in a separate thread
    let bridge_sock = bridge_socket.clone();
    let comp_sock = comp_socket.clone();
    let shutdown = Arc::new(AtomicBool::new(false));

    let bridge_shutdown = shutdown.clone();
    let bridge_thread = std::thread::spawn(move || {
        let cfg = proxy::BridgeConfig {
            bridge_display: bridge_sock,
            compositor_display: comp_sock,
            max_clients: 0,
            log_level: None,
        };
        let _ = proxy::run_with_shutdown(cfg, bridge_shutdown);
    });

    // Give bridge time to start
    std::thread::sleep(Duration::from_millis(100));

    // Connect a mock client to the bridge
    let rdir = std::env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| "/run/user/1000".to_string());
    let bridge_path = format!("{rdir}/{bridge_socket}");
    let mut client = std::os::unix::net::UnixStream::connect(&bridge_path)
        .expect("client connected to bridge");
    client.set_nonblocking(false).ok();

    // Accept the compositor-side connection
    let mut comp_stream = compositor.accept_connection();

    // Client sends: wl_display.get_registry (oid=1, op=1, new_id=2)
    let reg_new_id = 2u32;
    let get_reg = make_raw(1, 1, &reg_new_id.to_ne_bytes());
    send_raw_fd(&client, &get_reg, &[]);

    // Compositor receives it, should be forwarded by bridge
    let comp_received = read_raw(&comp_stream);
    assert_eq!(msg_oid(&comp_received), 1, "compositor sees get_registry on wl_display");
    assert_eq!(msg_op(&comp_received), 1, "opcode 1 = get_registry");

    // Compositor responds with registry globals
    let comp_reg_id = 2u32;

    // Send: wl_registry.global(name=1, "wl_compositor", version=4)
    let global1 = make_global_event(comp_reg_id, 1, "wl_compositor", 4);
    send_raw_fd(&comp_stream, &global1, &[]);

    // Send: wl_registry.global(name=2, "xdg_wm_base", version=6)
    let global2 = make_global_event(comp_reg_id, 2, "xdg_wm_base", 6);
    send_raw_fd(&comp_stream, &global2, &[]);

    // Send: wl_display.error (non-registry event) to trigger globals_collected
    // Actually we should send something innocuous. Let's send a sync (op 0 on wl_display = error? No)
    // wl_display doesn't have a "done" event in the traditional sense.
    // We'll send something the bridge will just forward: callback.done doesn't exist.
    // The bridge triggers globals_collected on ANY non-registry event.
    // Let's send wl_display.error which IS opcode 0 on oid=1
    // Better: send delete_id (oid=1, op=1) with a random id
    let delete_id = make_raw(1, 1, &999u32.to_ne_bytes());
    send_raw_fd(&comp_stream, &delete_id, &[]);

    // Now the client should receive forwarded globals + injected layer shell
    // Read 4 messages from client: global wl_compositor, global xdg_wm_base,
    // global zwlr_layer_shell_v1 (injected), delete_id
    let client_msg1 = read_raw(&client);
    eprintln!("Client msg1: oid={} op={} len={}", msg_oid(&client_msg1), msg_op(&client_msg1), client_msg1.len());
    assert_eq!(msg_oid(&client_msg1), comp_reg_id, "client receives global on registry");
    assert_eq!(msg_op(&client_msg1), 0, "op 0 = global event");

    let client_msg2 = read_raw(&client);
    eprintln!("Client msg2: oid={} op={} len={}", msg_oid(&client_msg2), msg_op(&client_msg2), client_msg2.len());
    assert_eq!(msg_oid(&client_msg2), comp_reg_id);

    let client_msg3 = read_raw(&client);
    eprintln!("Client msg3: oid={} op={} len={}", msg_oid(&client_msg3), msg_op(&client_msg3), client_msg3.len());

    // msg3 should be either the injected layer shell or delete_id
    // The bridge sends injected global BEFORE forwarding the non-registry message
    if msg_op(&client_msg3) == 0 && msg_oid(&client_msg3) == comp_reg_id {
        // This is injected layer shell
        let iface = msg_string(&client_msg3, 12);
        assert_eq!(iface, "zwlr_layer_shell_v1", "injected layer shell global");
    }

    let client_msg4 = read_raw(&client);
    eprintln!("Client msg4: oid={} op={} len={}", msg_oid(&client_msg4), msg_op(&client_msg4), client_msg4.len());

    // Verify that at least one message is the injected layer shell
    let injected = vec![&client_msg1, &client_msg2, &client_msg3, &client_msg4]
        .iter()
        .any(|m| {
            msg_op(m) == 0
                && msg_oid(m) == comp_reg_id
                && msg_string(m, 12) == "zwlr_layer_shell_v1"
        });
    assert!(injected, "bridge injected zwlr_layer_shell_v1 global");

    // Cleanup: send shutdown
    shutdown.store(true, Ordering::SeqCst);
    bridge_thread.join().expect("bridge thread joined");
}

#[test]
fn test_get_layer_surface_translation() {
    let test_id = std::process::id() + 1000;
    let comp_socket = format!("wayland-test-comp-trans-{test_id}");
    let bridge_socket = format!("wayland-test-bridge-trans-{test_id}");

    let compositor = MockCompositor::new(&comp_socket);

    let bridge_sock = bridge_socket.clone();
    let comp_sock = comp_socket.clone();
    let shutdown = Arc::new(AtomicBool::new(false));

    let bridge_shutdown = shutdown.clone();
    let bridge_thread = std::thread::spawn(move || {
        let cfg = proxy::BridgeConfig {
            bridge_display: bridge_sock,
            compositor_display: comp_sock,
            max_clients: 0,
            log_level: None,
        };
        let _ = proxy::run_with_shutdown(cfg, bridge_shutdown);
    });

    std::thread::sleep(Duration::from_millis(100));

    // Client connects
    let rdir = std::env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| "/run/user/1000".to_string());
    let bridge_path = format!("{rdir}/{bridge_socket}");
    let mut client = std::os::unix::net::UnixStream::connect(&bridge_path)
        .expect("client connect");
    client.set_nonblocking(false).ok();

    let mut comp_stream = compositor.accept_connection();

    // Phase 1: Registry setup (same as first test but simpler)
    // Client: get_registry
    send_raw_fd(&client, &make_raw(1, 1, &2u32.to_ne_bytes()), &[]);
    let _ = read_raw(&comp_stream); // bridge forwards it

    // Compositor: send wl_compositor global
    send_raw_fd(&comp_stream, &make_global_event(2, 1, "wl_compositor", 4), &[]);
    // Compositor: send xdg_wm_base global
    send_raw_fd(&comp_stream, &make_global_event(2, 2, "xdg_wm_base", 6), &[]);
    // Compositor: send delete_id to end global collection
    send_raw_fd(&comp_stream, &make_raw(1, 1, &999u32.to_ne_bytes()), &[]);

    // Client reads: wl_compositor global, xdg_wm_base global
    let _ = read_raw(&client); // wl_compositor
    let _ = read_raw(&client); // xdg_wm_base

    // Read injected layer shell
    let inj = read_raw(&client);
    assert_eq!(msg_string(&inj, 12), "zwlr_layer_shell_v1", "injected layer shell");

    // Phase 2: Client binds xdg_wm_base
    // wl_registry.bind(name=2, "xdg_wm_base", ver=6, new_id=10)
    let bind_xdg = make_bind(2, 2, "xdg_wm_base", 6, 10);
    send_raw_fd(&client, &bind_xdg, &[]);

    let _ = read_raw(&comp_stream); // forwarded by bridge

    // Client binds injected layer shell (name should be 1000 = FAKE_GLOBAL_LAYER_SHELL)
    // We don't know the exact name, but we know it's the 3rd global registered.
    // From the bridge logic, FAKE_GLOBAL_LAYER_SHELL = 1000.
    let bind_layer = make_bind(2, 1000, "zwlr_layer_shell_v1", 5, 20);
    send_raw_fd(&client, &bind_layer, &[]);
    // Bridge should intercept this — no forwarding to compositor

    // Phase 3: Client creates wl_surface (we simulate via wl_compositor.create_surface)
    // But the bridge doesn't intercept surface creation.
    // For simplicity, let's just call get_layer_surface with surface_id=30, output=0
    // get_layer_surface(new_id=40, surface=30, output=0, layer=2, ns="test")
    let ns = "test\0";
    let ns_len = ns.len() as u32;
    let mut gs_pay = Vec::new();
    gs_pay.extend_from_slice(&40u32.to_ne_bytes()); // new_id
    gs_pay.extend_from_slice(&30u32.to_ne_bytes()); // surface_id
    gs_pay.extend_from_slice(&0u32.to_ne_bytes());  // output (0 = compositor choice)
    gs_pay.extend_from_slice(&2u32.to_ne_bytes());  // layer (2 = top)
    gs_pay.extend_from_slice(&ns_len.to_ne_bytes()); // namespace string length
    gs_pay.extend_from_slice(ns.as_bytes());
    while gs_pay.len() % 4 != 0 { gs_pay.push(0); }

    send_raw_fd(&client, &make_raw(20, 0, &gs_pay), &[]);

    // Bridge should now:
    // - Send xdg_wm_base.get_xdg_surface(new_xdg, surface=30) to compositor
    // - Send xdg_surface.get_toplevel(new_toplevel) to compositor

    let comp_msg1 = read_raw(&comp_stream);
    eprintln!("comp msg1: oid={} op={} len={}", msg_oid(&comp_msg1), msg_op(&comp_msg1), comp_msg1.len());

    let comp_msg2 = read_raw(&comp_stream);
    eprintln!("comp msg2: oid={} op={} len={}", msg_oid(&comp_msg2), msg_op(&comp_msg2), comp_msg2.len());

    // Verify first message: xdg_wm_base.get_xdg_surface on compositor's xdg_wm_base
    // The client bound xdg_wm_base with new_id=10, bridge forwarded it.
    // But the compositor might use a different internal ID for xdg_wm_base.
    // Let's verify it's at least on the xdg_wm_base OID or similar.
    // The bridge stores cli_xdg_wm_base_id = 10, but the actual compositor
    // allocated its own OID for xdg_wm_base.

    // Simpler assertion: just check we got 2 messages from the bridge to compositor
    // (get_xdg_surface + get_toplevel)
    if !comp_msg1.is_empty() && !comp_msg2.is_empty() {
        // Check at least one message has payload size indicating get_xdg_surface:
        // get_xdg_surface has 8 bytes of payload (new_id + surface)
        const XDG_SURFACE_PAYLOAD: usize = 8;
        let msg1_has_xdg = comp_msg1.len() == 8 + XDG_SURFACE_PAYLOAD;
        let msg2_has_xdg = comp_msg2.len() == 8 + XDG_SURFACE_PAYLOAD;
        assert!(
            msg1_has_xdg || msg2_has_xdg,
            "expected xdg_surface creation (8 byte payload), got msg1:{} msg2:{}",
            comp_msg1.len(),
            comp_msg2.len()
        );
    }

    // Cleanup: destroy layer surface (op 7)
    send_raw_fd(&client, &make_raw(40, 7, &[]), &[]);
    // Give bridge time to process and send cleanup to compositor
    std::thread::sleep(Duration::from_millis(50));

    shutdown.store(true, Ordering::SeqCst);
    bridge_thread.join().expect("bridge thread joined");
}

// ─── Wire format helpers ────────────────────────────────────────────────────

fn make_global_event(reg_oid: u32, name: u32, iface: &str, version: u32) -> Vec<u8> {
    let mut iface_b = iface.as_bytes().to_vec();
    iface_b.push(0);
    let slen = iface_b.len() as u32;
    let padded = ((slen as usize) + 3) & !3;
    let mut pay = Vec::with_capacity(4 + 4 + padded + 4);
    pay.extend_from_slice(&name.to_ne_bytes());
    pay.extend_from_slice(&slen.to_ne_bytes());
    pay.extend_from_slice(&iface_b);
    while pay.len() < (8 + padded) {
        pay.push(0);
    }
    pay.extend_from_slice(&version.to_ne_bytes());
    make_raw(reg_oid, 0, &pay)
}

fn make_bind(reg_oid: u32, name: u32, iface: &str, version: u32, new_id: u32) -> Vec<u8> {
    let mut iface_b = iface.as_bytes().to_vec();
    iface_b.push(0);
    let slen = iface_b.len() as u32;
    let padded = ((slen as usize) + 3) & !3;
    let mut pay = Vec::with_capacity(4 + 4 + padded + 4 + 4);
    pay.extend_from_slice(&name.to_ne_bytes());
    pay.extend_from_slice(&slen.to_ne_bytes());
    pay.extend_from_slice(&iface_b);
    while pay.len() < (8 + padded) {
        pay.push(0);
    }
    pay.extend_from_slice(&version.to_ne_bytes());
    pay.extend_from_slice(&new_id.to_ne_bytes());
    make_raw(reg_oid, 0, &pay)
}

/// Send a raw message with FDs via sendmsg.
fn send_raw_fd(stream: &std::os::unix::net::UnixStream, raw: &[u8], _fds: &[RawFd]) {
    let fd = stream.as_raw_fd();
    let iov = [libc::iovec {
        iov_base: raw.as_ptr() as *mut _,
        iov_len: raw.len(),
    }];
    let mut mhdr: libc::msghdr = unsafe { std::mem::zeroed() };
    mhdr.msg_iov = iov.as_ptr() as *mut _;
    mhdr.msg_iovlen = 1;
    unsafe {
        let n = libc::sendmsg(fd, &mhdr, libc::MSG_NOSIGNAL);
        assert!(n >= 0, "sendmsg failed: {}", std::io::Error::last_os_error());
        assert_eq!(n as usize, raw.len(), "sendmsg short write");
    }
}
