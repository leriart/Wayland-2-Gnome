use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;

use anyhow::{bail, Result};
use log::{debug, error, info};

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
}

/// Send one complete Wayland wire message (data + optional FDs) via sendmsg.
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

        // If FDs arrived with payload (unusual), merge into our cbuf
        if pay_msg.msg_controllen > 0 {
            cbuf = pay_cbuf;
        }
    }

    // Extract FDs from header (or payload if header had none)
    let fds = if cbuf.iter().any(|&b| b != 0) {
        let mut fds = Vec::new();
        let mut ch = unsafe { libc::CMSG_FIRSTHDR(&hdr_msg) };
        // If no FDs in header, check in pay_msg structures — simplified: just check cbuf
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

// ─── State ──────────────────────────────────────────────────────────────────

/// Information about a global as advertised by the compositor.
#[derive(Clone, Debug)]
struct GlobalInfo {
    name: u32,
    interface: String,
    version: u32,
}

struct Session {
    to_cli: UnixStream,
    to_comp: UnixStream,
    /// Globals we've sniffed from compositor→client registry events.
    comp_globals: Vec<GlobalInfo>,
    /// The client's registry object ID (set by get_registry).
    cli_reg_id: u32,
    /// The registry object ID on the compositor side (same as cli_reg_id).
    comp_reg_id: u32,
    /// Whether we've finished collecting globals (got a non-registry event).
    globals_collected: bool,
}

// ─── Public entry point ─────────────────────────────────────────────────────

pub fn run(cfg: BridgeConfig) -> Result<()> {
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
        let (cli, _) = match lis.accept() {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(10));
                continue;
            }
            Err(e) => {
                error!("Accept: {e}");
                break Ok(());
            }
        };
        cli.set_nonblocking(false)?;
        info!("New client connected");

        let comp_display = cfg.compositor_display.clone();
        let rd2 = rdir.clone();

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
                    error!("Session error: {e}");
                }
                info!("Session done");
            })?;
    }
}

// ─── Config ─────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct BridgeConfig {
    pub bridge_display: String,
    pub compositor_display: String,
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            bridge_display: "wayland-bridge-0".into(),
            compositor_display: std::env::var("WAYLAND_DISPLAY")
                .unwrap_or_else(|_| "wayland-0".into()),
        }
    }
}

// ─── Session loop ───────────────────────────────────────────────────────────

fn session(to_cli: UnixStream, to_comp: UnixStream) -> Result<()> {
    let mut s = Session {
        to_cli,
        to_comp,
        comp_globals: Vec::new(),
        cli_reg_id: 0,
        comp_reg_id: 0,
        globals_collected: false,
    };

    let cfd = s.to_cli.as_raw_fd();
    let ofd = s.to_comp.as_raw_fd();

    loop {
        let mut pfds = [
            libc::pollfd {
                fd: cfd,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: ofd,
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        let ret = unsafe { libc::poll(pfds.as_mut_ptr(), 2, 1000) };
        if ret < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            bail!("poll failed: {}", std::io::Error::last_os_error());
        }

        if pfds[0].revents & (libc::POLLNVAL | libc::POLLHUP | libc::POLLERR) != 0 {
            info!("Client closed");
            return Ok(());
        }
        if pfds[1].revents & (libc::POLLNVAL | libc::POLLHUP | libc::POLLERR) != 0 {
            info!("Compositor closed");
            return Ok(());
        }

        // Collect phase: only read from compositor until globals are done
        if !s.globals_collected && s.comp_reg_id > 0 && pfds[1].revents & libc::POLLIN != 0 {
            let msg = match read_raw(&s.to_comp) {
                Ok(m) => m,
                Err(e) => {
                    info!("Compositor EOF: {e}");
                    return Ok(());
                }
            };

            // Forward to client first, then sniff
            send_raw(&s.to_cli, &msg)?;

            if msg.object_id() == s.comp_reg_id && msg.opcode() == 0 {
                // Registry global event — collect
                let pay = &msg.raw[8..];
                if pay.len() >= 12 {
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
                    s.comp_globals.push(GlobalInfo {
                        name: gname,
                        interface: iface,
                        version,
                    });
                }
            } else {
                // Non-registry event → globals collection done
                s.globals_collected = true;
                info!("globals collected: {} total", s.comp_globals.len());
            }
            continue;
        }

        // Normal forwarding: read client → compositor
        if pfds[0].revents & libc::POLLIN != 0 {
            let msg = match read_raw(&s.to_cli) {
                Ok(m) => m,
                Err(e) => {
                    info!("Client EOF: {e}");
                    return Ok(());
                }
            };
            handle_cli(&mut s, &msg)?;
        }

        // Normal forwarding: read compositor → client
        if pfds[1].revents & libc::POLLIN != 0 {
            let msg = match read_raw(&s.to_comp) {
                Ok(m) => m,
                Err(e) => {
                    info!("Compositor EOF: {e}");
                    return Ok(());
                }
            };
            send_raw(&s.to_cli, &msg)?;
        }
    }
}

// ─── Client messages ────────────────────────────────────────────────────────

fn handle_cli(s: &mut Session, msg: &RawMsg) -> Result<()> {
    let oid = msg.object_id();

    debug!("cli ➔ comp: oid={}, op={}, size={}", oid, msg.opcode(), msg.raw.len());

    // wl_display.get_registry (oid=1, op=1) — forward, then enable sniffing
    if oid == 1 && msg.opcode() == 1 {
        let new_id = if msg.raw.len() >= 12 {
            u32::from_ne_bytes([msg.raw[8], msg.raw[9], msg.raw[10], msg.raw[11]])
        } else {
            2
        };
        info!("get_registry → cli_reg={new_id}, comp_reg={new_id}");
        s.cli_reg_id = new_id;
        s.comp_reg_id = new_id;
        send_raw(&s.to_comp, msg)?;
        return Ok(());
    }

    // Registry bind — log it, then forward
    if s.cli_reg_id > 0 && oid == s.cli_reg_id && msg.opcode() == 0 {
        let _ = log_bind_and_forward(s, msg);
        return Ok(());
    }

    // Everything else: forward raw
    send_raw(&s.to_comp, msg)
}

fn log_bind_and_forward(s: &Session, msg: &RawMsg) -> Result<()> {
    let pay = &msg.raw[8..];
    if pay.len() >= 12 {
        let global_name = u32::from_ne_bytes([pay[0], pay[1], pay[2], pay[3]]);
        let new_id = u32::from_ne_bytes([pay[4], pay[5], pay[6], pay[7]]);
        let global = s.comp_globals.iter().find(|g| g.name == global_name);
        let iface = global.map(|g| g.interface.as_str()).unwrap_or("?");
        info!("bind: name={global_name}, new_id={new_id}, iface={iface}");
    }
    send_raw(&s.to_comp, msg)
}
