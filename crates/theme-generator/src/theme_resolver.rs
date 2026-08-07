//! Resolving the active Plasma theme directory.

use std::path::{Path, PathBuf};

use crate::Error;

/// A resolved Plasma theme.
#[derive(Debug, Clone)]
pub struct Theme {
    /// Theme name.
    pub name: String,
    /// Theme directory.
    pub dir: PathBuf,
    /// Path of the theme's `colors` file, if any.
    pub colors_file: Option<PathBuf>,
    /// Whether the theme requests blur behind its panels.
    pub blur_behind: bool,
}

/// Search paths for Plasma desktop themes, in priority order.
fn theme_search_paths() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = Vec::new();
    if let Some(config) = dirs::config_dir() {
        paths.push(config.join("plasma/desktoptheme"));
    }
    if let Some(data_home) = std::env::var_os("XDG_DATA_HOME") {
        paths.push(PathBuf::from(data_home).join("plasma/desktoptheme"));
    }
    if let Some(data_dirs) = std::env::var_os("XDG_DATA_DIRS") {
        for dir in std::env::split_paths(&data_dirs) {
            paths.push(dir.join("plasma/desktoptheme"));
        }
    }
    if let Some(data) = dirs::data_dir() {
        paths.push(data.join("plasma/desktoptheme"));
    }
    paths
}

/// Find the directory of a theme by name.
pub fn find_theme_dir(name: &str) -> Option<PathBuf> {
    theme_search_paths()
        .into_iter()
        .map(|base| base.join(name))
        .find(|dir| dir.is_dir())
}

/// Determine the active Plasma theme name.
///
/// Mirrors `Plasma::Theme`'s default resolution: the `currentTheme` key of
/// `plasmarc`, falling back to `default`.
pub fn active_theme_name() -> String {
    if let Some(plasmarc) = dirs::config_dir()
        && let Ok(text) = std::fs::read_to_string(plasmarc.join("plasmarc"))
        && let Some(name) = read_key(&text, "Theme", "currentTheme")
    {
        return name;
    }
    "default".to_owned()
}

/// Read a `key=value` from a `KConfig` file, optionally within a section.
fn read_key(text: &str, section: &str, key: &str) -> Option<String> {
    let mut in_section = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_section = line.trim_start_matches('[').trim_end_matches(']').trim() == section;
            continue;
        }
        if in_section
            && let Some((k, v)) = line.split_once('=')
            && k.trim() == key
        {
            let v = v.trim().trim_matches('"');
            if !v.is_empty() {
                return Some(v.to_owned());
            }
        }
    }
    None
}

/// Whether the theme requests blur behind its panels.
///
/// `Plasma::Theme::blurBehindEnabled()` defaults to true; a theme may opt
/// out through its metadata. We honor the `AdaptiveTransparency` / blur
/// keys we can find and otherwise default to true, matching the observed
/// behavior for the `default` and Ant-Dark themes.
fn theme_blur_behind(dir: &Path) -> bool {
    for file in ["metadata.json", "metadata.desktop"] {
        let path = dir.join(file);
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        // Explicit blur opt-out (metadata.desktop [Theme] blurBehind=false).
        if file == "metadata.desktop"
            && let Some(value) = read_key(&text, "Theme", "blurBehind")
        {
            return value == "true" || value == "1";
        }
        // metadata.json does not carry a blur flag; fall through.
        let _ = text;
    }
    true
}

/// Resolve a theme: explicit name if given, otherwise the active theme.
pub fn resolve_theme(name: Option<&str>) -> Result<Theme, Error> {
    let name = name.map_or_else(active_theme_name, str::to_owned);
    let dir = find_theme_dir(&name).ok_or_else(|| Error::other(format!("theme not found: {name}")))?;
    let colors_file = dir.join("colors");
    let colors_file = colors_file.is_file().then_some(colors_file);
    let blur_behind = theme_blur_behind(&dir);
    Ok(Theme {
        name,
        dir,
        colors_file,
        blur_behind,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_kconfig_key() {
        let text = "[Theme]\ncurrentTheme=Ant-Dark\n[Wallpapers]\nfoo=bar\n";
        assert_eq!(read_key(text, "Theme", "currentTheme"), Some("Ant-Dark".to_owned()));
        assert_eq!(read_key(text, "Theme", "missing"), None);
    }

    #[test]
    fn default_theme_name() {
        assert_eq!(active_theme_name(), "default");
    }
}
