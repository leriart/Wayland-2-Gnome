//! Wayland GNOME Bridge — Phase 4
//!
//! Selective byte-level proxy between client and compositor.
//! Intercepts ONLY `zwlr_layer_shell_v1` and passes everything else through.

use anyhow::Result;
use log::info;

mod proxy;

fn main() -> Result<()> {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info"),
    )
    .init();

    let config = proxy::BridgeConfig::default();
    info!(
        "Wayland 2 GNOME on '{}', proxying to '{}'",
        format!("/run/user/1000/{}", config.bridge_display),
        config.compositor_display
    );

    proxy::run(config)
}
