use std::io::IsTerminal;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use clap::Parser;
use futures_util::StreamExt;
use theme_generator::colors::Color;
use tokio::signal;
use tokio::time::sleep;
use tracing::{error, info, warn};
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

/// Outcome of comparing an incoming `ColorScheme` value with the last seen one.
enum ColorSchemeChange {
    /// The value differs from the last seen one; a regeneration is needed.
    Changed(String),
    /// The value equals the last seen one; skip.
    Unchanged,
    /// The value could not be parsed as a string; cannot compare.
    Unknown,
}

/// Outcome of comparing an incoming `accent-color` value with the last seen one.
enum AccentColorChange {
    /// The value differs from the last seen one; a regeneration is needed.
    Changed(Color),
    /// The value equals the last seen one; skip.
    Unchanged,
    /// The value could not be parsed; cannot compare.
    Unknown,
}

/// Parse the desktop accent color reported by the portal: a struct of three
/// doubles in `[0, 1]` (red, green, blue).
#[allow(
    clippy::cast_possible_truncation,
    reason = "channels are clamped to [0, 1] before scaling to 8-bit, so truncation cannot lose data"
)]
fn parse_accent_color(value: &zbus::zvariant::OwnedValue) -> Option<Color> {
    let structure = zbus::zvariant::Structure::try_from(&**value).ok()?;
    let mut fields = structure.fields().iter();
    let r = fields.next()?.downcast_ref::<f64>().ok()?;
    let g = fields.next()?.downcast_ref::<f64>().ok()?;
    let b = fields.next()?.downcast_ref::<f64>().ok()?;
    if fields.next().is_some() {
        return None;
    }
    Some(Color::opaque(
        (r.clamp(0.0, 1.0) * 255.0).round() as u8,
        (g.clamp(0.0, 1.0) * 255.0).round() as u8,
        (b.clamp(0.0, 1.0) * 255.0).round() as u8,
    ))
}

/// Compare an incoming `accent-color` setting value against the previously
/// seen value, updating the cache when the value differs.
fn classify_accent_color(
    value: &zbus::zvariant::OwnedValue,
    last: &mut Option<Color>,
) -> AccentColorChange {
    let Some(accent) = parse_accent_color(value) else {
        return AccentColorChange::Unknown;
    };
    if *last == Some(accent) {
        AccentColorChange::Unchanged
    } else {
        *last = Some(accent);
        AccentColorChange::Changed(accent)
    }
}

/// Process one `SettingChanged` signal; returns whether a regeneration
/// should be scheduled.
fn handle_setting_changed(
    args: &SettingChangedArgs<'_>,
    last_color_scheme: &mut Option<String>,
    last_accent_color: &mut Option<Color>,
) -> bool {
    if args.namespace == "org.kde.kdeglobals.General" && args.key == "ColorScheme" {
        match classify_color_scheme(&args.value, last_color_scheme) {
            ColorSchemeChange::Changed(scheme) => {
                info!(%scheme, "color-scheme changed");
                true
            }
            ColorSchemeChange::Unchanged => {
                info!("color-scheme unchanged, skipping");
                false
            }
            ColorSchemeChange::Unknown => {
                warn!("unable to parse color-scheme value, regenerating");
                true
            }
        }
    } else if args.namespace == "org.freedesktop.appearance" && args.key == "accent-color" {
        match classify_accent_color(&args.value, last_accent_color) {
            AccentColorChange::Changed(accent) => {
                info!(r = accent.r, g = accent.g, b = accent.b, "accent-color changed");
                true
            }
            AccentColorChange::Unchanged => {
                info!("accent-color unchanged, skipping");
                false
            }
            AccentColorChange::Unknown => {
                warn!("unable to parse accent-color value, regenerating");
                true
            }
        }
    } else {
        false
    }
}

/// Compare an incoming `ColorScheme` setting value against the previously
/// seen value, updating the cache when the value differs.
fn classify_color_scheme(
    value: &zbus::zvariant::OwnedValue,
    last: &mut Option<String>,
) -> ColorSchemeChange {
    let Ok(scheme) = value.downcast_ref::<&str>() else {
        return ColorSchemeChange::Unknown;
    };
    let scheme = scheme.to_owned();
    if last.as_deref() == Some(scheme.as_str()) {
        ColorSchemeChange::Unchanged
    } else {
        *last = Some(scheme.clone());
        ColorSchemeChange::Changed(scheme)
    }
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let wait_duration = Duration::from_millis(args.wait_time_ms);

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("fcitx_autotheme=info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(std::io::stdout().is_terminal())
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
    let mut last_color_scheme: Option<String> = None;
    let mut last_accent_color: Option<Color> = None;

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
                    regenerate_and_reload(&conn, last_color_scheme.as_deref(), last_accent_color)
                        .await;
                    triggered = false;
                }

                msg = portal_stream.next() => {
                    match msg {
                        Some(signal_msg) => {
                            match signal_msg.args() {
                                Ok(args) => {
                                    // Any signal during debounce restarts the
                                    // timer by re-looping below.
                                    handle_setting_changed(
                                        &args,
                                        &mut last_color_scheme,
                                        &mut last_accent_color,
                                    );
                                }
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
                                Ok(args) => {
                                    if handle_setting_changed(
                                        &args,
                                        &mut last_color_scheme,
                                        &mut last_accent_color,
                                    ) {
                                        triggered = true;
                                    }
                                }
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
async fn regenerate_and_reload(
    conn: &zbus::Connection,
    color_scheme: Option<&str>,
    accent_color: Option<Color>,
) {
    if let Err(e) = handle_theme_update(color_scheme, accent_color) {
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

/// Regenerate the fcitx5 theme from the current Plasma theme via the
/// in-process generator, honoring the cached color scheme and accent color.
fn handle_theme_update(
    color_scheme: Option<&str>,
    accent_color: Option<Color>,
) -> anyhow::Result<()> {
    let output_dir = theme_output_dir()?;
    let mut generator = theme_generator::ThemeGenerator::new(&output_dir);
    if let Some(scheme) = color_scheme {
        generator = generator.with_color_scheme_name(scheme);
    }
    if let Some(accent) = accent_color {
        generator = generator.with_accent_color(accent);
    }
    let generated = generator.generate()?;
    info!(
        "theme regenerated at {}: {}",
        output_dir.display(),
        generated.files.join(", ")
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Build the `variant struct { double, double, double }` value the portal
    /// sends for `accent-color`.
    fn accent_value(r: f64, g: f64, b: f64) -> zbus::zvariant::OwnedValue {
        zbus::zvariant::Value::Structure(zbus::zvariant::Structure::from((r, g, b)))
            .try_to_owned()
            .expect("owned value")
    }

    #[test]
    fn parses_accent_color_value() {
        let value = accent_value(0.662_745, 0.192_157, 0.266_667);
        assert_eq!(
            parse_accent_color(&value),
            Some(Color::opaque(169, 49, 68))
        );
    }

    #[test]
    fn accent_color_is_clamped_and_rounded() {
        let value = accent_value(-0.5, 0.5, 1.5);
        assert_eq!(parse_accent_color(&value), Some(Color::opaque(0, 128, 255)));
    }

    #[test]
    fn rejects_non_structure_accent_value() {
        let value = zbus::zvariant::Value::U8(42)
            .try_to_owned()
            .expect("owned value");
        assert_eq!(parse_accent_color(&value), None);
    }

    #[test]
    fn rejects_accent_value_with_wrong_arity() {
        let value = zbus::zvariant::Value::Structure(zbus::zvariant::Structure::from((0.5, 0.5)))
            .try_to_owned()
            .expect("owned value");
        assert_eq!(parse_accent_color(&value), None);
    }

    #[test]
    fn classifies_accent_color_changes() {
        let value = accent_value(0.66, 0.19, 0.27);
        let mut last = None;
        assert!(matches!(
            classify_accent_color(&value, &mut last),
            AccentColorChange::Changed(_)
        ));
        assert!(matches!(
            classify_accent_color(&value, &mut last),
            AccentColorChange::Unchanged
        ));
        assert_eq!(last, Some(Color::opaque(168, 48, 69)));
        let other = accent_value(0.0, 0.0, 0.0);
        assert!(matches!(
            classify_accent_color(&other, &mut last),
            AccentColorChange::Changed(_)
        ));
    }
}
