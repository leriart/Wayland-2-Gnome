//! Wayland GNOME Bridge — Phase 4
//!
//! Selective byte-level proxy between client and compositor.
//! Intercepts ONLY `zwlr_layer_shell_v1` and passes everything else through.

use std::os::unix::io::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use log::info;

use wayland_2_gnome::proxy;

#[derive(Parser)]
#[command(name = "wayland-2-gnome", about = "Protocol-aware Wayland proxy translating wlr-layer-shell to GNOME Shell overlays")]
struct Cli {
    /// Run as a background daemon (forks, detaches, writes PID file)
    #[arg(long)]
    daemon: bool,

    /// Bridge socket name (default: wayland-bridge-0)
    #[arg(long, default_value = "wayland-bridge-0")]
    socket: String,

    /// Compositor socket name (default: wayland-0)
    #[arg(long, default_value = "wayland-0")]
    compositor: String,

    /// Path to TOML config file
    #[arg(long)]
    config: Option<String>,

    /// Install systemd user service and exit
    #[arg(long)]
    install: bool,
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info"),
    )
    .init();

    let cli = Cli::parse();
    // --install: auto-install systemd user service
    if cli.install {
        return install_systemd_user_service(&cli.socket, &cli.compositor, cli.config.as_deref());
    }

    // Daemonize BEFORE signal setup to avoid issues with threaded env_logger

    // Daemonize BEFORE signal setup to avoid issues with threaded env_logger
    if cli.daemon {
        daemonize()?;
        // Re-init logging after fork because env_logger may have buffered state
        env_logger::Builder::from_env(
            env_logger::Env::default().default_filter_or("info"),
        )
        .init();
    }

    let shutdown = Arc::new(AtomicBool::new(false));
    install_signal_handlers(shutdown.clone())?;

    // Load config: start with defaults, optionally merge from file, then CLI overrides
    let mut config = proxy::BridgeConfig::default();

    if let Some(ref cfg_path) = cli.config {
        let file_cfg = proxy::BridgeConfig::from_file(cfg_path)?;
        config.merge(&file_cfg);
        info!("Loaded config from {}", cfg_path);
    }

    // CLI --socket and --compositor override config file and default
    config.bridge_display = cli.socket;
    config.compositor_display = cli.compositor;

    // Apply log level from config if set
    if let Some(ref level) = config.log_level {
        env_logger::Builder::from_env(
            env_logger::Env::default().default_filter_or(level),
        )
        .init();
        info!("Log level set to '{}' from config", level);
    }

    let sock_name = config.bridge_display.clone();

    info!(
        "Wayland 2 GNOME on '{}', proxying to '{}'",
        format!("/run/user/1000/{}", config.bridge_display),
        config.compositor_display
    );

    // Block until shutdown
    proxy::run_with_shutdown(config, shutdown)?;

    // Clean up PID file if daemon
    if cli.daemon {
        let rdir = std::env::var("XDG_RUNTIME_DIR")
            .unwrap_or_else(|_| "/run/user/1000".to_string());
        let pid_path = format!("{rdir}/{sock_name}.pid");
        let _ = std::fs::remove_file(&pid_path);
    }

    info!("Wayland 2 GNOME shutdown complete");
    Ok(())
}

/// Install systemd --user service unit and enable it.
fn install_systemd_user_service(socket: &str, compositor: &str, config_path: Option<&str>) -> Result<()> {
    let home = std::env::var("HOME").map_err(|_| anyhow::anyhow!("HOME not set"))?;
    let systemd_dir = format!("{home}/.config/systemd/user");
    let service_path = format!("{systemd_dir}/wayland-2-gnome.service");

    std::fs::create_dir_all(&systemd_dir)?;
    eprintln!("  [1/4] Created {systemd_dir}");

    let service_content = format!(
        r###"[Unit]
Description=Wayland 2 GNOME protocol bridge (wlr-layer-shell -> GNOME)
Documentation=https://github.com/SrGcorp/Wayland-2-Gnome
After=graphical-session.target
PartOf=graphical-session.target

[Service]
Type=simple
ExecStart=%h/.cargo/bin/wayland-2-gnome --socket {socket} --compositor {compositor}{config_opt}
Restart=on-failure
RestartSec=5
Environment=RUST_LOG=info

[Install]
WantedBy=graphical-session.target
"###,
        socket = socket,
        compositor = compositor,
        config_opt = config_path.map(|p| format!(" --config {p}")).unwrap_or_default()
    );

    std::fs::write(&service_path, service_content)?;
    eprintln!("  [2/4] Wrote {service_path}");

    let status = std::process::Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status()?;
    if !status.success() {
        anyhow::bail!("systemctl --user daemon-reload failed (exit {status:?})");
    }
    eprintln!("  [3/4] systemctl --user daemon-reload OK");

    let status = std::process::Command::new("systemctl")
        .args(["--user", "enable", "wayland-2-gnome.service"])
        .status()?;
    if !status.success() {
        anyhow::bail!("systemctl --user enable failed (exit {status:?})");
    }
    eprintln!("  [4/4] systemctl --user enable OK");

    eprintln!();
    eprintln!("Service installed. Start it with:");
    eprintln!("  systemctl --user start wayland-2-gnome.service");
    eprintln!();
    eprintln!("Check status:");
    eprintln!("  systemctl --user status wayland-2-gnome.service");
    eprintln!();
    eprintln!("View logs:");
    eprintln!("  journalctl --user -u wayland-2-gnome.service -f");

    Ok(())
}

fn daemonize() -> Result<()> {
    use nix::unistd::{fork, ForkResult};

    match unsafe { fork() }? {
        ForkResult::Parent { child: _ } => {
            // Parent exits immediately
            std::process::exit(0);
        }
        ForkResult::Child => {
            // Child: create new session, detach from terminal
            nix::unistd::setsid()?;

            // Redirect stdin/stdout/stderr to /dev/null
            let null = std::fs::File::open("/dev/null")?;
            let null_fd = null.as_raw_fd();
            unsafe {
                libc::dup2(null_fd, 0); // stdin
                libc::dup2(null_fd, 1); // stdout
                libc::dup2(null_fd, 2); // stderr
            }

            // Write PID file
            let rdir = std::env::var("XDG_RUNTIME_DIR")
                .unwrap_or_else(|_| "/run/user/1000".to_string());
            let pid_path = format!("{rdir}/wayland-bridge-0.pid");
            let pid = std::process::id();
            std::fs::write(&pid_path, pid.to_string())?;

            // If WAYLAND_2_GNOME_LOG is set, redirect output there for debugging
            if let Ok(log_file) = std::env::var("WAYLAND_2_GNOME_LOG") {
                let f = std::fs::File::create(&log_file)?;
                let f_fd = f.as_raw_fd();
                unsafe {
                    libc::dup2(f_fd, 1);
                    libc::dup2(f_fd, 2);
                }
            }

            info!("Daemon started (PID {pid})");
            Ok(())
        }
    }
}

/// Install signal handlers for graceful shutdown on SIGTERM and SIGINT.
fn install_signal_handlers(_shutdown: Arc<AtomicBool>) -> Result<()> {
    use nix::sys::signal::{sigaction, SaFlags, SigAction, SigHandler, SigSet, Signal};

    // External C function that can be used as a sigaction handler
    extern "C" fn handle_signal(_sig: std::os::raw::c_int) {
        // Use the static atomic in the proxy module
        // SAFETY: Signal handlers are extremely constrained; we use a static AtomicBool.
        wayland_2_gnome::proxy::SHUTDOWN_FLAG.store(true, Ordering::SeqCst);
    }

    let handler = SigHandler::Handler(handle_signal);
    let sa = SigAction::new(handler, SaFlags::empty(), SigSet::empty());
    unsafe {
        sigaction(Signal::SIGTERM, &sa)?;
        sigaction(Signal::SIGINT, &sa)?;
        sigaction(Signal::SIGHUP, &sa)?;
    }

    info!("Signal handlers installed (SIGTERM, SIGINT, SIGHUP)");
    Ok(())
}


