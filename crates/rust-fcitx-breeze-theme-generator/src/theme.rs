// SPDX-FileCopyrightText: 2026 Fcitx Breeze Theme Generator Authors
//
// SPDX-License-Identifier: GPL-2.0-or-later
//
//! Plasma theme discovery and metadata (Plasma::Theme equivalent).
//!
//! Theme directories live under `plasma/desktoptheme/<name>` on every XDG data
//! root (`~/.local/share`, then `$XDG_DATA_DIRS`), mirroring
//! `QStandardPaths::locate(GenericDataLocation, "plasma/desktoptheme/<name>")`.

use crate::colorscheme::ColorScheme;
use std::path::{Path, PathBuf};

pub const DEFAULT_THEME: &str = "default";
const THEME_SUBDIR: &str = "plasma/desktoptheme";

/// A resolved Plasma theme.
#[derive(Debug, Clone)]
pub struct Theme {
    /// The theme id (directory name).
    name: String,
    /// Absolute path of the theme directory.
    dir: PathBuf,
    /// Resolved color scheme.
    pub colors: ColorScheme,
    /// Whether blur-behind is enabled (`[BlurBehindEffect] enabled`, default true).
    pub blur_behind_enabled: bool,
}

/// XDG data directories in search order.
fn data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("XDG_DATA_HOME") {
        dirs.push(PathBuf::from(home));
    } else if let Some(home) = std::env::var_os("HOME") {
        dirs.push(Path::new(&home).join(".local/share"));
    }
    if let Some(data_dirs) = std::env::var_os("XDG_DATA_DIRS") {
        for d in std::env::split_paths(&data_dirs) {
            if !d.as_os_str().is_empty() {
                dirs.push(d);
            }
        }
    } else {
        dirs.push(PathBuf::from("/usr/local/share"));
        dirs.push(PathBuf::from("/usr/share"));
    }
    dirs
}

/// Locate a theme directory by name, falling back to `default`.
fn locate_theme(name: &str) -> Option<(String, PathBuf)> {
    for candidate in [name.to_string(), DEFAULT_THEME.to_string()] {
        for root in data_dirs() {
            let p = root.join(THEME_SUBDIR).join(&candidate);
            if p.is_dir() {
                return Some((candidate, p));
            }
        }
    }
    None
}

impl Theme {
    /// Load a theme by name. Falls back to the `default` theme when `name` is
    /// missing or not installed (Plasma 5 behavior; KSvg's ImageSet also falls
    /// back to `default` for SVG files in Plasma 6).
    pub fn new(name: &str) -> Result<Self, String> {
        let (name, dir) = locate_theme(name)
            .ok_or_else(|| format!("could not locate plasma theme `{name}`"))?;

        // blur behind: [BlurBehindEffect] enabled=true from plasmarc or metadata.desktop
        let blur_behind_enabled = read_blur_behind(&dir);

        let colors_path = dir.join("colors");
        let colors = ColorScheme::load(
            colors_path.is_file().then_some(colors_path.as_path()),
            crate::colorscheme::kdeglobals_path().as_deref(),
        );

        Ok(Theme {
            name,
            dir,
            colors,
            blur_behind_enabled,
        })
    }

    /// The theme id (directory name) — what `Plasma::Theme::themeName()` returns.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The resolved theme directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Resolve an image path like `dialogs/background` to a file on disk.
    ///
    /// Tries `<theme>/<path>.svgz`, `<theme>/<path>.svg`, then the fallback
    /// `default` theme, exactly like `KSvg::ImageSet::imagePath`.
    pub fn image_path(&self, name: &str) -> Option<PathBuf> {
        if name.is_empty() || name.contains("../") {
            return None;
        }
        for theme_name in [self.name.as_str(), DEFAULT_THEME] {
            for root in data_dirs() {
                let base = root.join(THEME_SUBDIR).join(theme_name);
                for ext in ["svgz", "svg"] {
                    let p = base.join(format!("{name}.{ext}"));
                    if p.is_file() {
                        return Some(p);
                    }
                }
            }
        }
        None
    }
}

fn read_blur_behind(dir: &Path) -> bool {
    for file in ["plasmarc", "metadata.desktop"] {
        let p = dir.join(file);
        if let Ok(text) = std::fs::read_to_string(&p) {
            let mut in_group = false;
            for line in text.lines() {
                let line = line.trim();
                if line.starts_with('[') && line.ends_with(']') {
                    in_group = line == "[BlurBehindEffect]";
                } else if in_group && line.starts_with("enabled=") {
                    return line[8..].trim().eq_ignore_ascii_case("true");
                }
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_locate_default() {
        let theme = Theme::new("definitely-not-a-real-theme").unwrap();
        assert_eq!(theme.name(), DEFAULT_THEME);
        assert!(theme.dir.is_dir());
    }
}
