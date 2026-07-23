use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::Context;
use clap::Parser;
use futures_util::StreamExt;
use tokio::signal;
use tokio::time::sleep;
use tracing::{error, info};
use zbus::proxy;

/// CLI arguments.
#[derive(Parser)]
#[command(
    name = "fcitx-autotheme",
    about = "Watch Plasma theme changes and update fcitx5 theme"
)]
struct Args {
    /// Debounce wait time in milliseconds before processing
    #[arg(short = 'w', long = "wait-time", default_value = "100", value_name = "MILLIS")]
    wait_time_ms: u64,
}

/// Proxy trait for XDG Desktop Portal Settings interface.
///
/// Monitors `SettingChanged` signals emitted when desktop appearance
/// settings (like color-scheme) change.
#[proxy(
    interface = "org.freedesktop.portal.Settings",
    default_service = "org.freedesktop.portal.Desktop",
    default_path = "/org/freedesktop/portal/desktop"
)]
trait PortalSettings {
    /// Signal emitted when a setting changes.
    ///
    /// Parameters:
    /// - `namespace`: setting namespace (e.g. "org.kde.kdeglobals.General")
    /// - `key`: setting key (e.g. "ColorScheme")
    /// - `value`: new setting value (variant type)
    #[zbus(signal)]
    fn setting_changed(
        &self,
        namespace: String,
        key: String,
        value: zbus::zvariant::OwnedValue,
    ) -> zbus::Result<()>;
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let wait_duration = Duration::from_millis(args.wait_time_ms);

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("fcitx_autotheme=info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .try_init()
        .map_err(|e| anyhow::anyhow!("failed to initialize tracing subscriber: {e}"))?;

    info!(
        "fcitx-autotheme daemon started (debounce: {} ms)",
        args.wait_time_ms
    );

    let conn = zbus::Connection::session()
        .await
        .context("failed to connect to D-Bus session bus")?;

    let portal_proxy = PortalSettingsProxy::new(&conn)
        .await
        .context("failed to create portal settings proxy")?;

    let mut portal_stream = portal_proxy
        .receive_setting_changed()
        .await
        .context("failed to subscribe to SettingChanged signal")?;

    let kconfig_rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.kde.kconfig.notify")
        .context("failed to build match rule: invalid interface")?
        .member("ConfigChanged")
        .context("failed to build match rule: invalid member")?
        .path("/plasmarc")
        .context("failed to build match rule: invalid path")?
        .build();

    let mut kconfig_stream =
        zbus::MessageStream::for_match_rule(kconfig_rule, &conn, Some(1))
            .await
            .context("failed to subscribe to ConfigChanged signal")?;

    let mut triggered = false;

    'outer: loop {
        if triggered {
            // Debounce: sleep, drain signals to reset timer
            tokio::select! {
                biased;

                _ = shutdown_signal() => {
                    info!("shutdown signal received, exiting");
                    break 'outer;
                }

                _ = sleep(wait_duration) => {
                    info!("debounce elapsed, regenerating fcitx5 theme");
                    regenerate_and_reload(&conn).await;
                    triggered = false;
                }

                msg = portal_stream.next() => {
                    match msg {
                        Some(signal_msg) => {
                            match signal_msg.args() {
                                Ok(args)
                                    if args.namespace == "org.kde.kdeglobals.General"
                                        && args.key == "ColorScheme" =>
                                {
                                    // Signal during debounce: restart timer by re-looping
                                }
                                Ok(_) => {} // unrelated signal
                                Err(e) => error!(%e, "failed to parse signal args"),
                            }
                        }
                        None => {
                            info!("portal signal stream ended");
                            break 'outer;
                        }
                    }
                }

                msg = kconfig_stream.next() => {
                    match msg {
                        Some(Ok(_)) => {
                            // Signal during debounce: restart timer by re-looping
                        }
                        Some(Err(e)) => {
                            error!(%e, "error receiving kconfig signal");
                        }
                        None => {
                            info!("kconfig signal stream ended");
                            break 'outer;
                        }
                    }
                }
            }
        } else {
            // Idle: wait for first signal
            tokio::select! {
                biased;

                _ = shutdown_signal() => {
                    info!("shutdown signal received, exiting");
                    break 'outer;
                }

                msg = portal_stream.next() => {
                    match msg {
                        Some(signal_msg) => {
                            match signal_msg.args() {
                                Ok(args)
                                    if args.namespace == "org.kde.kdeglobals.General"
                                        && args.key == "ColorScheme" =>
                                {
                                    info!("color-scheme changed");
                                    triggered = true;
                                }
                                Ok(_) => {}
                                Err(e) => error!(%e, "failed to parse signal args"),
                            }
                        }
                        None => {
                            info!("portal signal stream ended");
                            break 'outer;
                        }
                    }
                }

                msg = kconfig_stream.next() => {
                    match msg {
                        Some(Ok(_)) => {
                            info!("Plasma config changed");
                            triggered = true;
                        }
                        Some(Err(e)) => {
                            error!(%e, "error receiving kconfig signal");
                        }
                        None => {
                            info!("kconfig signal stream ended");
                            break 'outer;
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// Regenerate theme and reload fcitx5 config, logging errors.
async fn regenerate_and_reload(conn: &zbus::Connection) {
    if let Err(e) = handle_theme_update().await {
        error!(%e, "theme update failed");
    }
    if let Err(e) = reload_fcitx5(conn).await {
        error!(%e, "fcitx5 config reload failed");
    }
}

/// Resolve the Flatpak fcitx5 theme output directory.
fn theme_output_dir() -> anyhow::Result<PathBuf> {
    let home = dirs::home_dir().context("failed to determine home directory")?;
    Ok(home.join(".var/app/org.fcitx.Fcitx5/data/fcitx5/themes/plasma"))
}

/// Run `fcitx5-plasma-theme-generator` to regenerate the fcitx5 theme
/// from the current Plasma color scheme.
async fn handle_theme_update() -> anyhow::Result<()> {
    let output_dir = theme_output_dir()?;

    let output = tokio::process::Command::new("fcitx5-plasma-theme-generator")
        .arg("-o")
        .arg(&output_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("failed to run fcitx5-plasma-theme-generator")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "fcitx5-plasma-theme-generator exited with status {}: {}",
            output.status,
            stderr.trim()
        );
    }

    info!("theme regenerated at {}", output_dir.display());
    Ok(())
}

/// Reload the fcitx5 classicui addon configuration via D-Bus.
async fn reload_fcitx5(conn: &zbus::Connection) -> anyhow::Result<()> {
    conn.call_method(
        Some("org.fcitx.Fcitx5"),
        "/controller",
        Some("org.fcitx.Fcitx.Controller1"),
        "ReloadAddonConfig",
        &("classicui"),
    )
    .await
    .context("failed to call ReloadAddonConfig on org.fcitx.Fcitx5")?;

    info!("fcitx5 addon config reloaded");
    Ok(())
}

/// Wait for a shutdown signal (SIGINT or SIGTERM).
async fn shutdown_signal() -> anyhow::Result<()> {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .context("failed to wait for SIGINT")
    };

    #[cfg(unix)]
    let terminate = wait_for_terminate();
    #[cfg(not(unix))]
    let terminate = std::future::pending::<anyhow::Result<()>>();

    tokio::select! {
        res = ctrl_c => { res?; }
        res = terminate => { res?; }
    }

    Ok(())
}

#[cfg(unix)]
async fn wait_for_terminate() -> anyhow::Result<()> {
    signal::unix::signal(signal::unix::SignalKind::terminate())
        .context("failed to install SIGTERM handler")?
        .recv()
        .await;
    Ok(())
}
