// SPDX-FileCopyrightText: 2026 Fcitx Breeze Theme Generator Authors
//
// SPDX-License-Identifier: GPL-2.0-or-later
//
//! The theme generator — a faithful port of the original C++
//! `WatcherApp::generateTheme()` from fcitx5-configtool.

use crate::color::Color;
use crate::ini::{write_ini, RawConfig};
use crate::render::{encode_png, Pixmap};
use crate::svg::{FrameSvg, Svg};
use crate::theme::Theme;
use std::path::{Path, PathBuf};

/// Icon sizes from `KIconLoader` used by the original generator.
pub const ICON_SIZE_SMALL: u32 = 16;
pub const ICON_SIZE_SMALL_MEDIUM: u32 = 22;

/// Result of a generation run.
#[derive(Debug, Default)]
pub struct GenerateResult {
    /// Files written, relative to the output directory.
    pub files: Vec<String>,
}

/// Generate an Fcitx 5 Classic UI theme from a Plasma theme.
///
/// Mirrors the original CLI contract: `theme` is the Plasma theme name,
/// `output_dir` is where `theme.conf` and the PNG assets are written.
pub fn generate_theme(theme: &Theme, output_dir: &Path) -> Result<GenerateResult, String> {
    let mut result = GenerateResult::default();
    std::fs::create_dir_all(output_dir).map_err(|e| format!("failed to create dir: {e}"))?;

    // Same logic from plasma-frameworks: gridUnit is the font "M" height.
    // The original derives it from QFontMetrics of the app font; we approximate
    // with the standard Plasma grid unit of 16px (QFontMetrics on typical
    // systems yields 16-17; 16 is the canonical Plasma default).
    let grid_unit = grid_unit();
    let small_spacing = (grid_unit / 4).max(2);
    let text_margin = small_spacing as f32 / 2.0;

    let mut config = RawConfig::new("");

    // Metadata
    {
        let metadata = config.sub_mut("Metadata");
        metadata.set("Name", "Plasma");
        metadata.set("Version", "1");
        metadata.set("Author", "Fcitx");
        metadata.set(
            "Description",
            format!("Theme generated from Plasma Theme {}", theme.name()),
        );
    }

    let input_panel = config.sub_mut("InputPanel");
    input_panel.set("NormalColor", theme_color(theme, "window_fg"));
    input_panel.set("HighlightCandidateColor", theme_color(theme, "window_fg"));
    input_panel.set("HighlightColor", theme_color(theme, "selection_fg"));
    input_panel.set("HighlightBackgroundColor", theme_color(theme, "selection_bg"));
    input_panel.set("PageButtonAlignment", "Last Candidate");
    input_panel.set("NormalColor", theme_color(theme, "window_fg"));
    input_panel.set("HighlightCandidateColor", theme_color(theme, "window_fg"));
    let _ = input_panel;
    config.sub_mut("Menu");

    let stylesheet = theme.colors.stylesheet();

    // ── Panel background (dialogs/background) ──────────────────────────────
    let mut background = Pixmap::new(200, 200);

    let (mut shadow_left, mut shadow_top, mut shadow_right, mut shadow_bottom) = (0.0, 0.0, 0.0, 0.0);
    let mut shadow_svg = FrameSvg::new();
    set_theme_to_svg(&mut shadow_svg, theme, &stylesheet, "dialogs/background");
    let has_shadow = shadow_svg.has_element_prefix("shadow");
    if has_shadow {
        shadow_svg.set_element_prefix("shadow");
        shadow_svg.resize_frame(200.0, 200.0);
        (shadow_left, shadow_top, shadow_right, shadow_bottom) = shadow_svg.get_margins();
    }

    let mut svg = FrameSvg::new();
    set_theme_to_svg(&mut svg, theme, &stylesheet, "dialogs/background");
    svg.resize_frame(
        200.0 - (shadow_left + shadow_right),
        200.0 - (shadow_top + shadow_bottom),
    );
    let (mut bg_left, mut bg_top, mut bg_right, mut bg_bottom) = svg.get_margins();

    // paint background frame at (shadowLeft, shadowTop), then shadow on top
    if let Some(frame) = svg.frame_pixmap() {
        background.draw_pixmap(shadow_left.round() as i32, shadow_top.round() as i32, &frame);
    }
    if has_shadow {
        if let Some(frame) = shadow_svg.frame_pixmap() {
            background.draw_pixmap(0, 0, &frame);
        }
    }

    bg_left += shadow_left;
    bg_top += shadow_top;
    bg_right += shadow_right;
    bg_bottom += shadow_bottom;

    save_png(&background, output_dir, "panel.png", &mut result)?;

    svg.resize_frame(
        200.0 - (shadow_left + shadow_right) - 2.0,
        200.0 - (shadow_top + shadow_bottom) - 2.0,
    );
    if theme.blur_behind_enabled {
        let mut mask = Pixmap::new(200, 200);
        if let Some(alpha) = svg.alpha_mask() {
            // QPixmap::mask() → 1-bit bitmap of fully opaque pixels, drawn at
            // (shadowLeft+1, shadowTop+1). Black where opaque, transparent elsewhere.
            let mask_bmp = bitmap_mask(&alpha);
            mask.draw_pixmap(
                (shadow_left + 1.0).round() as i32,
                (shadow_top + 1.0).round() as i32,
                &mask_bmp,
            );
        }
        save_png(&mask, output_dir, "mask.png", &mut result)?;
    }

    config.set("Menu/Spacing", (text_margin.round() as i64).to_string());
    set_margins_to_config(&mut config, "InputPanel/ContentMargin", bg_left, bg_top, bg_right, bg_bottom);
    set_margins_to_config(&mut config, "Menu/ContentMargin", bg_left, bg_top, bg_right, bg_bottom);
    set_margins_to_config(
        &mut config,
        "InputPanel/ShadowMargin",
        shadow_left,
        shadow_top,
        shadow_right,
        shadow_bottom,
    );

    config.set("InputPanel/Background/Image", "panel.png");
    if theme.blur_behind_enabled {
        config.set("InputPanel/BlurMask", "mask.png");
        config.set("InputPanel/EnableBlur", "True");
    }
    config.set("Menu/Background/Image", "panel.png");
    if theme.blur_behind_enabled {
        config.set("Menu/BlurMask", "mask.png");
        config.set("Menu/EnableBlur", "True");
    }
    set_margins_to_config(&mut config, "InputPanel/Background/Margin", bg_left, bg_top, bg_right, bg_bottom);
    set_margins_to_config(&mut config, "Menu/Background/Margin", bg_left, bg_top, bg_right, bg_bottom);

    // ── Highlight (widgets/viewitem) ───────────────────────────────────────
    {
        let mut highlight_svg = FrameSvg::new();
        set_theme_to_svg(&mut highlight_svg, theme, &stylesheet, "widgets/viewitem");
        if highlight_svg.has_element_prefix("hover") {
            highlight_svg.set_element_prefix("hover");
        } else if highlight_svg.has_element_prefix("selected") {
            highlight_svg.set_element_prefix("selected");
        }
        highlight_svg.resize_frame(200.0, 200.0);
        if let Some(pm) = highlight_svg.frame_pixmap() {
            save_png(&pm, output_dir, "highlight.png", &mut result)?;
        }

        let (mut hl_left, mut hl_top, mut hl_right, mut hl_bottom) = highlight_svg.get_margins();
        hl_left = hl_left.max(text_margin);
        hl_top = hl_top.max(text_margin);
        hl_right = hl_right.max(text_margin);
        hl_bottom = hl_bottom.max(text_margin);

        config.set("InputPanel/Highlight/Image", "highlight.png");
        config.set("Menu/Highlight/Image", "highlight.png");
        set_margins_to_config(
            &mut config,
            "InputPanel/Highlight/Margin",
            hl_left,
            hl_top,
            hl_right,
            hl_bottom,
        );
        set_margins_to_config(
            &mut config,
            "Menu/Highlight/Margin",
            hl_left,
            hl_top,
            hl_right,
            hl_bottom,
        );
        set_margins_to_config(
            &mut config,
            "InputPanel/TextMargin",
            hl_left,
            hl_top + text_margin,
            hl_right,
            hl_bottom + text_margin,
        );
        set_margins_to_config(
            &mut config,
            "Menu/TextMargin",
            hl_left,
            hl_top,
            hl_right,
            hl_bottom,
        );
    }

    // ── Icons: arrows / checkmarks / line ──────────────────────────────────
    {
        let mut icon = Svg::new();
        icon.set_contains_multiple_images(true);
        set_theme_to_svg(&mut icon, theme, &stylesheet, "widgets/arrows");
        icon.resize(ICON_SIZE_SMALL_MEDIUM as f32, ICON_SIZE_SMALL_MEDIUM as f32);
        if icon.has_element("left-arrow") && icon.has_element("right-arrow") {
            config.set("InputPanel/PrevPage/Image", "prev.png");
            if let Some(pm) = icon.pixmap("left-arrow") {
                save_png(&pm, output_dir, "prev.png", &mut result)?;
            }
            config.set("InputPanel/NextPage/Image", "next.png");
            if let Some(pm) = icon.pixmap("right-arrow") {
                save_png(&pm, output_dir, "next.png", &mut result)?;
            }
        }
        icon.resize(ICON_SIZE_SMALL as f32, ICON_SIZE_SMALL as f32);
        if icon.has_element("right-arrow") {
            config.set("Menu/SubMenu/Image", "arrow.png");
            if let Some(pm) = icon.pixmap("right-arrow") {
                save_png(&pm, output_dir, "arrow.png", &mut result)?;
            }
        }

        let mut radio = Svg::new();
        radio.set_contains_multiple_images(true);
        set_theme_to_svg(&mut radio, theme, &stylesheet, "widgets/checkmarks");
        radio.resize(ICON_SIZE_SMALL as f32, ICON_SIZE_SMALL as f32);
        if radio.has_element("radiobutton") {
            config.set("Menu/CheckBox/Image", "radio.png");
            if let Some(pm) = radio.pixmap("radiobutton") {
                save_png(&pm, output_dir, "radio.png", &mut result)?;
            }
        }

        let mut line = Svg::new();
        line.set_contains_multiple_images(true);
        set_theme_to_svg(&mut line, theme, &stylesheet, "widgets/line");
        if line.has_element("horizontal-line") {
            if let Some(pm) = line.pixmap("horizontal-line") {
                save_png(&pm, output_dir, "line.png", &mut result)?;
            }
            config.set("Menu/Separator/Image", "line.png");
        }
    }

    // ── theme.conf ─────────────────────────────────────────────────────────
    let mut conf = String::new();
    write_ini(&config, &mut conf);
    let conf_path = output_dir.join("theme.conf");
    std::fs::write(&conf_path, conf).map_err(|e| format!("failed to write theme.conf: {e}"))?;
    result.files.push("theme.conf".to_string());

    Ok(result)
}

/// `setMarginsToConfig` from the original.
fn set_margins_to_config(
    config: &mut RawConfig,
    path: &str,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
) {
    let node = config.get_mut(path);
    node.set("Left", (left.round() as i64).to_string());
    node.set("Top", (top.round() as i64).to_string());
    node.set("Right", (right.round() as i64).to_string());
    node.set("Bottom", (bottom.round() as i64).to_string());
}

fn theme_color(theme: &Theme, role: &str) -> String {
    let g = match role {
        "selection_fg" => &theme.colors.selection.fg_normal,
        "selection_bg" => &theme.colors.selection.bg_normal,
        _ => &theme.colors.window.fg_normal,
    };
    Color::from_rgba8(g.0, g.1, g.2, 255).to_hex_string()
}

fn set_theme_to_svg<T>(svg: &mut T, theme: &Theme, stylesheet: &str, image_path: &str)
where
    T: SvgTarget,
{
    let path = theme.image_path(image_path);
    if let Some(path) = path {
        svg.set_path(&path, stylesheet);
    }
}

trait SvgTarget {
    fn set_path(&mut self, path: &Path, stylesheet: &str);
}

impl SvgTarget for FrameSvg {
    fn set_path(&mut self, path: &Path, stylesheet: &str) {
        self.set_image_path(path, stylesheet);
    }
}

impl SvgTarget for Svg {
    fn set_path(&mut self, path: &Path, stylesheet: &str) {
        self.set_image_path(path, stylesheet);
    }
}

fn save_png(
    pixmap: &Pixmap,
    dir: &Path,
    name: &str,
    result: &mut GenerateResult,
) -> Result<(), String> {
    let data = encode_png(pixmap)?;
    let path = dir.join(name);
    std::fs::write(&path, data).map_err(|e| format!("failed to write {name}: {e}"))?;
    result.files.push(name.to_string());
    Ok(())
}

/// Convert a pixmap to a 1-bit mask (QPixmap::mask semantics): opaque pixels
/// become black, transparent pixels become transparent.
fn bitmap_mask(src: &Pixmap) -> Pixmap {
    let mut out = Pixmap::new(src.width, src.height);
    for (dst, px) in out.data.chunks_exact_mut(4).zip(src.data.chunks_exact(4)) {
        if px[3] > 0 {
            dst[..4].copy_from_slice(&[0, 0, 0, 255]);
        }
    }
    out
}

/// Approximate the Plasma `gridUnit` (QFontMetrics "M" height). Plasma's
/// standard grid unit is 16px; QFontMetrics on typical systems yields 16-17.
fn grid_unit() -> i32 {
    16
}

/// Default output directory: `~/.local/share/fcitx5/themes/plasma`
/// (`fcitx::StandardPaths::userDirectory(PkgData) / "themes/plasma"`).
pub fn default_output_path() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| Path::new(&h).join(".local/share"))
        })
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("fcitx5/themes/plasma")
}
