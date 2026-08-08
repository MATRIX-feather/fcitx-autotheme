// SPDX-FileCopyrightText: 2026 Fcitx Breeze Theme Generator Authors
//
// SPDX-License-Identifier: GPL-2.0-or-later
//
//! KColorScheme-equivalent color resolution.
//!
//! Replicates the exact role→key mapping and cascade used by KSvg's
//! `ImageSetPrivate::namedColor` / `svgStyleSheet` (which drive the SVG color
//! substitution) and by `Plasma::Theme::color` (which feeds `theme.conf`).
//!
//! Cascade for every key, matching `KSharedConfig::openConfig(colorsFile)` with
//! `FullConfig` flags (IncludeGlobals | CascadeConfig):
//! 1. the theme's `colors` file
//! 2. `kdeglobals` (the user's global color scheme)
//! 3. the hardcoded Breeze Light defaults from `kcolorscheme.cpp`

use crate::ini::RawConfig;
use std::path::Path;

/// One full color set (one `[Colors:*]` group).
#[derive(Debug, Clone, PartialEq)]
pub struct ColorGroup {
    pub fg_normal: (u8, u8, u8),
    pub fg_inactive: (u8, u8, u8),
    pub fg_active: (u8, u8, u8),
    pub fg_link: (u8, u8, u8),
    pub fg_visited: (u8, u8, u8),
    pub fg_negative: (u8, u8, u8),
    pub fg_neutral: (u8, u8, u8),
    pub fg_positive: (u8, u8, u8),
    pub bg_normal: (u8, u8, u8),
    pub bg_alternate: (u8, u8, u8),
    pub deco_focus: (u8, u8, u8),
    pub deco_hover: (u8, u8, u8),
}

fn parse_triplet(node: Option<&RawConfig>) -> Option<(u8, u8, u8)> {
    let v = node?.value();
    let mut parts = v.split(',');
    let r: u8 = parts.next()?.trim().parse().ok()?;
    let g: u8 = parts.next()?.trim().parse().ok()?;
    let b: u8 = parts.next()?.trim().parse().ok()?;
    Some((r, g, b))
}

// Hardcoded Breeze Light defaults from kcolorscheme.cpp (7 groups × 12 roles).
type GroupDefaults = [(u8, u8, u8); 12];
const DEFAULTS: [(&str, GroupDefaults); 7] = [
    (
        "Colors:Window",
        [
            (35, 38, 41),    // fg_normal
            (112, 125, 138), // fg_inactive
            (61, 174, 233),  // fg_active
            (41, 128, 185),  // fg_link
            (155, 89, 182),  // fg_visited
            (218, 68, 83),   // fg_negative
            (246, 116, 0),   // fg_neutral
            (39, 174, 96),   // fg_positive
            (239, 240, 241), // bg_normal
            (227, 229, 231), // bg_alternate
            (61, 174, 233),  // deco_focus
            (147, 206, 233), // deco_hover
        ],
    ),
    (
        "Colors:Selection",
        [
            (255, 255, 255),
            (112, 125, 138),
            (255, 255, 255),
            (253, 188, 75),
            (155, 89, 182),
            (176, 55, 69),
            (198, 92, 0),
            (23, 104, 57),
            (61, 174, 233),
            (163, 212, 250),
            (61, 174, 233),
            (147, 206, 233),
        ],
    ),
    (
        "Colors:Button",
        [
            (35, 38, 41),
            (112, 125, 138),
            (61, 174, 233),
            (41, 128, 185),
            (155, 89, 182),
            (218, 68, 83),
            (246, 116, 0),
            (39, 174, 96),
            (252, 252, 252),
            (163, 212, 250),
            (61, 174, 233),
            (147, 206, 233),
        ],
    ),
    (
        "Colors:View",
        [
            (35, 38, 41),
            (112, 125, 138),
            (61, 174, 233),
            (41, 128, 185),
            (155, 89, 182),
            (218, 68, 83),
            (246, 116, 0),
            (39, 174, 96),
            (255, 255, 255),
            (247, 247, 247),
            (61, 174, 233),
            (147, 206, 233),
        ],
    ),
    (
        "Colors:Complementary",
        [
            (252, 252, 252),
            (161, 169, 177),
            (61, 174, 233),
            (29, 153, 243),
            (155, 89, 182),
            (218, 68, 83),
            (246, 116, 0),
            (39, 174, 96),
            (42, 46, 50),
            (27, 30, 32),
            (61, 174, 233),
            (147, 206, 233),
        ],
    ),
    (
        "Colors:Header",
        [
            (35, 38, 41),
            (112, 125, 138),
            (61, 174, 233),
            (41, 128, 185),
            (155, 89, 182),
            (218, 68, 83),
            (246, 116, 0),
            (39, 174, 96),
            (222, 224, 226),
            (239, 240, 241),
            (61, 174, 233),
            (147, 206, 233),
        ],
    ),
    (
        "Colors:Tooltip",
        [
            (35, 38, 41),
            (112, 125, 138),
            (61, 174, 233),
            (41, 128, 185),
            (155, 89, 182),
            (218, 68, 83),
            (246, 116, 0),
            (39, 174, 96),
            (247, 247, 247),
            (239, 240, 241),
            (61, 174, 233),
            (147, 206, 233),
        ],
    ),
];

fn default_for(group: &str) -> ColorGroup {
    let arr = DEFAULTS
        .iter()
        .find(|(name, _)| *name == group)
        .map(|(_, v)| v)
        .unwrap();
    ColorGroup {
        fg_normal: arr[0],
        fg_inactive: arr[1],
        fg_active: arr[2],
        fg_link: arr[3],
        fg_visited: arr[4],
        fg_negative: arr[5],
        fg_neutral: arr[6],
        fg_positive: arr[7],
        bg_normal: arr[8],
        bg_alternate: arr[9],
        deco_focus: arr[10],
        deco_hover: arr[11],
    }
}

/// A fully-resolved color scheme (equivalent to the 7 `KColorScheme` instances
/// KSvg keeps in `ImageSetPrivate`).
#[derive(Debug, Clone)]
pub struct ColorScheme {
    pub window: ColorGroup,
    pub selection: ColorGroup,
    pub button: ColorGroup,
    pub view: ColorGroup,
    pub complementary: ColorGroup,
    pub header: ColorGroup,
    pub tooltip: ColorGroup,
    /// `0.1 * [KDE] contrast` (default 0.7 when the key is absent).
    pub contrast: f32,
    /// `[KDE] frameContrast`, clamped to [0,1], default 0.2.
    pub frame_contrast: f32,
}

/// Read an INI file into a `RawConfig` tree (group paths become sub items).
fn read_ini(path: &Path) -> Option<RawConfig> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut config = RawConfig::new("");
    let mut current_group = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            current_group = line[1..line.len() - 1].to_string();
        } else if let Some(eq) = line.find('=') {
            let key = line[..eq].trim();
            let value = line[eq + 1..].trim();
            if !current_group.is_empty() {
                let group_node = config.sub_mut(&current_group);
                group_node.set(key, value);
            }
        }
    }
    if config.has_sub_items() {
        Some(config)
    } else {
        None
    }
}

impl ColorScheme {
    /// Build the color scheme for a theme.
    ///
    /// `theme_colors_path`: the theme's `colors` file, if any.
    /// `kdeglobals_path`: `~/.config/kdeglobals`, if any.
    pub fn load(theme_colors_path: Option<&Path>, kdeglobals_path: Option<&Path>) -> Self {
        let theme = theme_colors_path.and_then(read_ini);
        let globals = kdeglobals_path.and_then(read_ini);

        let mut scheme = ColorScheme {
            window: default_for("Colors:Window"),
            selection: default_for("Colors:Selection"),
            button: default_for("Colors:Button"),
            view: default_for("Colors:View"),
            complementary: default_for("Colors:Complementary"),
            header: default_for("Colors:Header"),
            tooltip: default_for("Colors:Tooltip"),
            contrast: 0.7,
            frame_contrast: 0.2,
        };

        scheme.window = resolve_group(theme.as_ref(), globals.as_ref(), "Colors:Window");
        scheme.selection = resolve_group(theme.as_ref(), globals.as_ref(), "Colors:Selection");
        scheme.button = resolve_group(theme.as_ref(), globals.as_ref(), "Colors:Button");
        scheme.view = resolve_group(theme.as_ref(), globals.as_ref(), "Colors:View");
        scheme.complementary =
            resolve_group(theme.as_ref(), globals.as_ref(), "Colors:Complementary");
        scheme.header = resolve_group(theme.as_ref(), globals.as_ref(), "Colors:Header");
        scheme.tooltip = resolve_group(theme.as_ref(), globals.as_ref(), "Colors:Tooltip");

        scheme.contrast = 0.1
            * resolve_kde_float(theme.as_ref(), globals.as_ref(), "contrast")
                .unwrap_or(7.0)
                .clamp(0.0, 10.0);
        scheme.frame_contrast = resolve_kde_float(theme.as_ref(), globals.as_ref(), "frameContrast")
            .unwrap_or(0.2)
            .clamp(0.0, 1.0);

        scheme
    }

    /// The `ColorScheme-*` stylesheet used for SVG color substitution,
    /// replicating `ImageSetPrivate::svgStyleSheet` for the Window color set
    /// with Normal status (which is what the generator uses).
    pub fn stylesheet(&self) -> String {
        let mut css = String::new();
        macro_rules! push {
            ($class:literal, $color:expr) => {
                css.push_str(&format!(
                    ".ColorScheme-{}{{color:#{:02x}{:02x}{:02x};}}",
                    $class, $color.0, $color.1, $color.2
                ));
            };
        }
        let window = &self.window;
        let selection = &self.selection;
        let button = &self.button;
        let view = &self.view;
        let tooltip = &self.tooltip;
        let complementary = &self.complementary;
        let header = &self.header;

        push!("Text", window.fg_normal);
        push!("Background", window.bg_normal);
        push!("Highlight", selection.bg_normal);
        push!("HighlightedText", selection.fg_normal);
        push!("PositiveText", window.fg_positive);
        push!("NeutralText", window.fg_neutral);
        push!("NegativeText", window.fg_negative);

        push!("ButtonText", button.fg_normal);
        push!("ButtonBackground", button.bg_normal);
        push!("ButtonHover", button.deco_hover);
        push!("ButtonFocus", button.deco_focus);
        push!("ButtonHighlightedText", selection.fg_normal);
        push!("ButtonPositiveText", button.fg_positive);
        push!("ButtonNeutralText", button.fg_neutral);
        push!("ButtonNegativeText", button.fg_negative);

        push!("ViewText", view.fg_normal);
        push!("ViewBackground", view.bg_normal);
        push!("ViewHover", view.deco_hover);
        push!("ViewFocus", view.deco_focus);
        push!("ViewHighlightedText", selection.fg_normal);
        push!("ViewPositiveText", view.fg_positive);
        push!("ViewNeutralText", view.fg_neutral);
        push!("ViewNegativeText", view.fg_negative);

        push!("TooltipText", tooltip.fg_normal);
        push!("TooltipBackground", tooltip.bg_normal);
        push!("TooltipHover", tooltip.deco_hover);
        push!("TooltipFocus", tooltip.deco_focus);
        push!("TooltipHighlightedText", selection.fg_normal);
        push!("TooltipPositiveText", tooltip.fg_positive);
        push!("TooltipNeutralText", tooltip.fg_neutral);
        push!("TooltipNegativeText", tooltip.fg_negative);

        push!("ComplementaryText", complementary.fg_normal);
        push!("ComplementaryBackground", complementary.bg_normal);
        push!("ComplementaryHover", complementary.deco_hover);
        push!("ComplementaryFocus", complementary.deco_focus);
        push!("ComplementaryHighlightedText", selection.fg_normal);
        push!("ComplementaryPositiveText", complementary.fg_positive);
        push!("ComplementaryNeutralText", complementary.fg_neutral);
        push!("ComplementaryNegativeText", complementary.fg_negative);

        push!("HeaderText", header.fg_normal);
        push!("HeaderBackground", header.bg_normal);
        push!("HeaderHover", header.deco_hover);
        push!("HeaderFocus", header.deco_focus);
        push!("HeaderHighlightedText", selection.fg_normal);
        push!("HeaderPositiveText", header.fg_positive);
        push!("HeaderNeutralText", header.fg_neutral);
        push!("HeaderNegativeText", header.fg_negative);

        // Frame = mix(bg, fg, frameContrast) — linear premultiplied mix
        let frame = mix_rgb(window.bg_normal, window.fg_normal, self.frame_contrast);
        push!("Frame", frame);

        css
    }
}

/// `KColorUtils::mix(c1, c2, bias)` for opaque colors.
fn mix_rgb(c1: (u8, u8, u8), c2: (u8, u8, u8), bias: f32) -> (u8, u8, u8) {
    if bias <= 0.0 {
        return c1;
    }
    if bias >= 1.0 {
        return c2;
    }
    let mix = |a: u8, b: u8| -> u8 {
        let v = a as f32 + (b as f32 - a as f32) * bias;
        v.round() as u8
    };
    (mix(c1.0, c2.0), mix(c1.1, c2.1), mix(c1.2, c2.2))
}

/// Resolve one color group with the theme → kdeglobals → defaults cascade.
fn resolve_group(
    theme: Option<&RawConfig>,
    globals: Option<&RawConfig>,
    group: &str,
) -> ColorGroup {
    let mut result = default_for(group);
    let theme_section = theme.and_then(|t| t.get(group));
    let globals_section = globals.and_then(|g| g.get(group));

    macro_rules! pick {
        ($field:ident, $key:literal) => {
            if let Some(v) = theme_section
                .and_then(|s| parse_triplet(s.child($key)))
                .or_else(|| globals_section.and_then(|s| parse_triplet(s.child($key))))
            {
                result.$field = v;
            }
        };
    }
    pick!(fg_normal, "ForegroundNormal");
    pick!(fg_inactive, "ForegroundInactive");
    pick!(fg_active, "ForegroundActive");
    pick!(fg_link, "ForegroundLink");
    pick!(fg_visited, "ForegroundVisited");
    pick!(fg_negative, "ForegroundNegative");
    pick!(fg_neutral, "ForegroundNeutral");
    pick!(fg_positive, "ForegroundPositive");
    pick!(bg_normal, "BackgroundNormal");
    pick!(bg_alternate, "BackgroundAlternate");
    pick!(deco_focus, "DecorationFocus");
    pick!(deco_hover, "DecorationHover");
    result
}

fn resolve_kde_float(
    theme: Option<&RawConfig>,
    globals: Option<&RawConfig>,
    key: &str,
) -> Option<f32> {
    theme
        .and_then(|t| t.get("KDE"))
        .and_then(|k| k.child(key))
        .and_then(|n| n.value().trim().parse::<f32>().ok())
        .or_else(|| {
            globals
                .and_then(|g| g.get("KDE"))
                .and_then(|k| k.child(key))
                .and_then(|n| n.value().trim().parse::<f32>().ok())
        })
}

/// Locate kdeglobals using `$XDG_CONFIG_HOME` or `~/.config`.
pub fn kdeglobals_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".config")))?;
    let p = base.join("kdeglobals");
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defaults() {
        let scheme = ColorScheme::load(None, None);
        assert_eq!(scheme.window.fg_normal, (35, 38, 41));
        assert_eq!(scheme.window.bg_normal, (239, 240, 241));
        assert_eq!(scheme.selection.bg_normal, (61, 174, 233));
        assert_eq!(scheme.selection.fg_normal, (255, 255, 255));
    }

    #[test]
    fn test_stylesheet_contains_frame() {
        let scheme = ColorScheme::load(None, None);
        let css = scheme.stylesheet();
        assert!(css.contains(".ColorScheme-Background{color:#eff0f1;}"));
        assert!(css.contains(".ColorScheme-Text{color:#232629;}"));
        assert!(css.contains(".ColorScheme-Highlight{color:#3daee9;}"));
        assert!(css.contains(".ColorScheme-Frame{color:"));
    }

    #[test]
    fn test_read_ini() {
        let dir = std::env::temp_dir().join(format!("fcitx-breeze-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("colors");
        std::fs::write(
            &p,
            "[Colors:Window]\nForegroundNormal=10,20,30\nBackgroundNormal=40,50,60\n\n[KDE]\ncontrast=4\n",
        )
        .unwrap();
        let scheme = ColorScheme::load(Some(&p), None);
        assert_eq!(scheme.window.fg_normal, (10, 20, 30));
        assert_eq!(scheme.window.bg_normal, (40, 50, 60));
        assert_eq!(scheme.window.fg_inactive, (112, 125, 138));
        assert!((scheme.contrast - 0.4).abs() < 0.001);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
