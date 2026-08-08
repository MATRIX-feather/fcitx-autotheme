//! Generate fcitx5 classicui themes from a Plasma theme.
//!
//! This crate is a Rust reimplementation of the `fcitx5-plasma-theme-generator`
//! tool (upstream: fcitx/fcitx5-configtool, `src/plasmathemegenerator/main.cpp`,
//! GPL-2.0-or-later). It reproduces the same algorithm: resolve the active
//! Plasma theme's color scheme, render the theme's SVG frames to PNGs, and
//! write a `theme.conf` in fcitx5 classicui format.
//!
//! Images can also be emitted as SVGs ([`OutputFormat::Svg`]): the theme's
//! frames are filtered and normalized into vector documents that fcitx5
//! renders itself, so panels scale to any size without rasterization loss.

#![deny(missing_docs)]
// The pipeline works with 8-bit RGBA channels and f32 geometry; the pedantic
// cast lints are inherent to image rasterization, so they are relaxed
// crate-wide (the workspace already relaxes other domain-specific lints).
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_lossless,
    clippy::cast_precision_loss,
    clippy::cast_possible_wrap,
    clippy::multiple_crate_versions,
    reason = "8-bit image channels and f32 geometry require these casts; duplicate transitive versions come from image 0.25"
)]

pub mod colors;
pub mod frame;
pub mod svg;
pub mod theme_conf;
pub mod theme_resolver;

use std::path::{Path, PathBuf};

use colors::{style_sheet, Color, ColorScheme, ThemeConfColors};
use image::RgbaImage;
use svg::PlasmaSvg;
use theme_resolver::Theme;

/// Errors produced by theme generation.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Filesystem error.
    #[error("I/O error on {path}: {source}")]
    Io {
        /// The path that failed.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
    /// Invalid color value in a config file.
    #[error("invalid color value {value:?} for {key} in {path}")]
    InvalidColor {
        /// The file containing the bad value.
        path: PathBuf,
        /// The key.
        key: String,
        /// The raw value.
        value: String,
    },
    /// A required SVG element is missing.
    #[error("required element {element} not found in {path}")]
    MissingElement {
        /// The SVG file.
        path: PathBuf,
        /// The element id.
        element: String,
    },
    /// SVG parse or render error.
    #[error("SVG error in {path}: {source}")]
    Svg {
        /// The SVG file.
        path: PathBuf,
        /// The underlying error.
        source: usvg::Error,
    },
    /// Pixmap allocation failure.
    #[error("failed to allocate {width}x{height} image")]
    Alloc {
        /// Width.
        width: u32,
        /// Height.
        height: u32,
    },
    /// Other failure.
    #[error("{0}")]
    Other(String),
}

impl Error {
    const fn io(path: PathBuf, source: std::io::Error) -> Self {
        Self::Io { path, source }
    }

    fn invalid_color(path: &std::path::Path, key: &str, value: &str) -> Self {
        Self::InvalidColor {
            path: path.to_path_buf(),
            key: key.to_owned(),
            value: value.to_owned(),
        }
    }

    const fn svg(path: PathBuf, source: usvg::Error) -> Self {
        Self::Svg { path, source }
    }

    fn missing_element(path: PathBuf, element: &str) -> Self {
        Self::MissingElement {
            path,
            element: element.to_owned(),
        }
    }

    fn other(message: impl Into<String>) -> Self {
        Self::Other(message.into())
    }
}

/// Result alias for theme generation.
pub type Result<T> = std::result::Result<T, Error>;

/// Size of the generated panel/mask/highlight images.
const FRAME_SIZE: u32 = 200;
/// Icon size `KIconLoader::SizeSmallMedium`.
const ICON_SIZE_SMALL_MEDIUM: u32 = 22;
/// Icon size `KIconLoader::SizeSmall`.
const ICON_SIZE_SMALL: u32 = 16;

/// Output image format for the generated theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputFormat {
    /// Rasterized PNGs (the classic behavior; works on every fcitx5 version).
    #[default]
    Png,
    /// Vector SVGs. fcitx5 renders them natively since mid-2026 (master);
    /// older releases rasterize them via gdk-pixbuf at the SVG's intrinsic
    /// size, so smaller frames lose sharpness there.
    Svg,
}

/// A theme generation run result.
#[derive(Debug)]
pub struct GeneratedTheme {
    /// The output directory.
    pub dir: PathBuf,
    /// Names of the files that were written.
    pub files: Vec<String>,
}

/// Generates fcitx5 classicui themes from a Plasma theme.
///
/// Mirrors `fcitx5-plasma-theme-generator` (one-shot mode).
#[derive(Debug, Clone)]
pub struct ThemeGenerator {
    output_dir: PathBuf,
    theme_name: Option<String>,
    /// `QFontMetrics("M").height()` replacement (Plasma grid unit at 96 dpi).
    grid_unit: u32,
    /// Override for the active color scheme (tests / custom setups).
    colors_file: Option<PathBuf>,
    /// Color scheme name from the watcher, pinned when it can be resolved.
    color_scheme_name: Option<String>,
    /// Desktop accent color override for the Highlight role.
    accent_color: Option<Color>,
    /// Deepen highlight-driven colors (decoration roles + Highlight) by this
    /// many percent (0 = unchanged): darkened and saturated by the same
    /// percentage.
    highlight_deepen_percent: u8,
    /// Image format of the generated theme.
    format: OutputFormat,
}

impl ThemeGenerator {
    /// Create a generator writing into `output_dir`.
    #[must_use]
    pub fn new(output_dir: impl Into<PathBuf>) -> Self {
        Self {
            output_dir: output_dir.into(),
            theme_name: None,
            grid_unit: 19,
            colors_file: None,
            color_scheme_name: None,
            accent_color: None,
            highlight_deepen_percent: 10,
            format: OutputFormat::Png,
        }
    }

    /// Pin the Plasma theme by name; by default the active theme is used.
    #[must_use]
    pub fn with_theme_name(mut self, name: impl Into<String>) -> Self {
        self.theme_name = Some(name.into());
        self
    }

    /// Override the active color scheme with a `.colors` file.
    #[must_use]
    pub fn with_colors_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.colors_file = Some(path.into());
        self
    }

    /// Pin the color scheme by name; by default the active scheme is used.
    /// When the name cannot be resolved on disk, the active scheme is used.
    #[must_use]
    pub fn with_color_scheme_name(mut self, name: impl Into<String>) -> Self {
        self.color_scheme_name = Some(name.into());
        self
    }

    /// Override the theme's Highlight (accent) color. When absent, the color
    /// scheme's own highlight color is used.
    #[must_use]
    pub const fn with_accent_color(mut self, accent: Color) -> Self {
        self.accent_color = Some(accent);
        self
    }

    /// Deepen the highlight-driven colors by `percent` percent (0 = unchanged,
    /// 10 = 10% darker and 10% more saturated).
    ///
    /// Applies to the decoration roles (`.ColorScheme-*Focus`/`*Hover`,
    /// coloring highlight.png and radio.png) and the `Highlight` role
    /// (theme.conf `HighlightBackgroundColor`), in both the rendered PNGs and
    /// the config colors. Defaults to 10.
    #[must_use]
    pub const fn with_highlight_deepening(mut self, percent: u8) -> Self {
        self.highlight_deepen_percent = percent;
        self
    }

    /// Override the grid unit (default 19, matching 96 dpi Plasma).
    #[must_use]
    pub const fn with_grid_unit(mut self, grid_unit: u32) -> Self {
        self.grid_unit = grid_unit;
        self
    }

    /// Set the output image format (default [`OutputFormat::Png`]).
    #[must_use]
    pub const fn with_format(mut self, format: OutputFormat) -> Self {
        self.format = format;
        self
    }

    /// Generate the theme into the output directory.
    pub fn generate(&self) -> Result<GeneratedTheme> {
        let theme = theme_resolver::resolve_theme(self.theme_name.as_deref())?;

        // SVG rendering uses the active color scheme (KColorScheme), while
        // theme.conf colors honor the theme's own `colors` file when present
        // (Plasma::Theme). Both fall back to the active scheme.
        let active_scheme = match &self.colors_file {
            Some(path) => ColorScheme::from_file(path)?,
            None => colors::load_scheme_for_name(self.color_scheme_name.as_deref())?,
        };
        let theme_conf_scheme = match &theme.colors_file {
            Some(path) => ColorScheme::from_file(path)?,
            None => active_scheme.clone(),
        };
        // The desktop accent color overrides the Highlight role in both the
        // SVG stylesheet and the theme.conf colors.
        let (active_scheme, theme_conf_scheme) = match self.accent_color {
            Some(accent) => (
                active_scheme.with_accent_color(accent),
                theme_conf_scheme.with_accent_color(accent),
            ),
            None => (active_scheme, theme_conf_scheme),
        };
        // Deepen highlight-driven colors (decoration roles + Highlight) in both
        // the stylesheet (PNGs) and the theme.conf colors.
        let (active_scheme, theme_conf_scheme) = (
            active_scheme.with_highlight_deepening(self.highlight_deepen_percent),
            theme_conf_scheme.with_highlight_deepening(self.highlight_deepen_percent),
        );
        let theme_colors: ThemeConfColors = theme_conf_scheme.theme_conf_colors();
        let css = style_sheet(&active_scheme);

        std::fs::create_dir_all(&self.output_dir)
            .map_err(|source| Error::io(self.output_dir.clone(), source))?;

        let mut config = theme_conf::Config::new();
        // Config keys are set in the same order as main.cpp so the serialized
        // tree matches the reference output (Metadata, InputPanel, Menu, then
        // each section's children depth-first).
        self.init_config(&mut config, &theme, &theme_colors);

        let mut files: Vec<String> = Vec::new();
        match self.format {
            OutputFormat::Png => {
                self.generate_panel(&theme, &css, &mut config, &mut files)?;
                self.generate_highlight(&theme, &css, &mut config, &mut files)?;
                self.generate_icons(&theme, &css, &mut config, &mut files)?;
            }
            OutputFormat::Svg => {
                self.generate_panel_svg(&theme, &css, &mut config, &mut files)?;
                self.generate_highlight_svg(&theme, &css, &mut config, &mut files)?;
                self.generate_icons_svg(&theme, &css, &mut config, &mut files)?;
            }
        }

        let path = self.output_dir.join("theme.conf");
        let text = config.serialize();
        std::fs::write(&path, text).map_err(|source| Error::io(path.clone(), source))?;
        files.push("theme.conf".to_owned());

        Ok(GeneratedTheme {
            dir: self.output_dir.clone(),
            files,
        })
    }

    /// Metadata and input-panel colors (main.cpp lines 186-205), plus the
    /// Menu spacing (line 280).
    fn init_config(
        &self,
        config: &mut theme_conf::Config,
        theme: &Theme,
        colors: &ThemeConfColors,
    ) {
        config.set("Metadata/Name", "Plasma");
        config.set("Metadata/Version", "1");
        config.set("Metadata/Author", "Fcitx");
        config.set(
            "Metadata/Description",
            &format!("Theme generated from Plasma Theme {}", theme.name),
        );
        config.set("InputPanel/NormalColor", colors.normal.to_fcitx_string().as_str());
        config.set(
            "InputPanel/HighlightCandidateColor",
            colors.normal.to_fcitx_string().as_str(),
        );
        config.set("InputPanel/HighlightColor", colors.highlighted.to_fcitx_string().as_str());
        config.set(
            "InputPanel/HighlightBackgroundColor",
            colors.highlight_background.to_fcitx_string().as_str(),
        );
        config.set("InputPanel/PageButtonAlignment", "Last Candidate");
        config.set("Menu/Spacing", &format!("{:.6}", self.text_margin()));
    }

    /// Panel background: bg frame + shadow frame + optional blur mask.
    fn generate_panel(
        &self,
        theme: &Theme,
        css: &str,
        config: &mut theme_conf::Config,
        files: &mut Vec<String>,
    ) -> Result<RgbaImage> {
        let mut background = PlasmaSvg::load(theme, "dialogs/background", css)?;

        let has_shadow = background.has_element_prefix("shadow");
        let shadow_margins = if has_shadow {
            let frame = background.frame("shadow", FRAME_SIZE, FRAME_SIZE)?;
            frame.margins()
        } else {
            (0.0, 0.0, 0.0, 0.0)
        };
        let (shadow_left, shadow_top, shadow_right, shadow_bottom) = shadow_margins;

        let bg_size = FRAME_SIZE - (shadow_left + shadow_right) as u32;
        let bg_size_h = FRAME_SIZE - (shadow_top + shadow_bottom) as u32;
        let (bg, bg_margins) = {
            let frame = background.frame("", bg_size, bg_size_h)?;
            let margins = frame.margins();
            let image = frame.render()?;
            (image, margins)
        };

        let mut panel = RgbaImage::new(FRAME_SIZE, FRAME_SIZE);
        paste(&mut panel, &bg, shadow_left as u32, shadow_top as u32);
        if has_shadow {
            let frame = background.frame("shadow", FRAME_SIZE, FRAME_SIZE)?;
            let shadow = frame.render()?;
            composite_over(&mut panel, &shadow, 0, 0);
        }
        write_png(&self.output_dir, "panel.png", &panel, files)?;

        // main.cpp lines 256-259: the bg frame is drawn offset by the shadow
        // frame, so ContentMargin and Background/Margin measure from the
        // panel's outer edge (bg margin + shadow offset).
        let (bg_left, bg_top, bg_right, bg_bottom) = bg_margins;
        let content_margins = (
            bg_left + shadow_left,
            bg_top + shadow_top,
            bg_right + shadow_right,
            bg_bottom + shadow_bottom,
        );
        set_margins(config, "InputPanel", "ContentMargin", content_margins);
        set_margins(config, "Menu", "ContentMargin", content_margins);
        set_margins(config, "InputPanel", "ShadowMargin", shadow_margins);
        config.set("InputPanel/Background/Image", "panel.png");
        config.set("Menu/Background/Image", "panel.png");
        set_margins(config, "InputPanel/Background", "Margin", content_margins);
        set_margins(config, "Menu/Background", "Margin", content_margins);

        if theme.blur_behind {
            let mask = generate_mask(&mut background, shadow_left, shadow_top)?;
            write_png(&self.output_dir, "mask.png", &mask, files)?;
            config.set("InputPanel/BlurMask", "mask.png");
            config.set("InputPanel/EnableBlur", "True");
        }

        Ok(panel)
    }

    /// Candidate highlight image from `widgets/viewitem` (hover or selected).
    fn generate_highlight(
        &self,
        theme: &Theme,
        css: &str,
        config: &mut theme_conf::Config,
        files: &mut Vec<String>,
    ) -> Result<()> {
        let mut viewitem = PlasmaSvg::load(theme, "widgets/viewitem", css)?;
        let prefix = if viewitem.has_element_prefix("hover") {
            "hover"
        } else if viewitem.has_element_prefix("selected") {
            "selected"
        } else {
            return Ok(());
        };
        let frame = viewitem.frame(prefix, FRAME_SIZE, FRAME_SIZE)?;
        let (l, t, r, b) = frame.margins();
        let image = frame.render()?;
        write_png(&self.output_dir, "highlight.png", &image, files)?;
        // Mirror main.cpp: margins floored at the text margin.
        let text_margin = self.text_margin();
        let (l, t, r, b) = (
            l.max(text_margin),
            t.max(text_margin),
            r.max(text_margin),
            b.max(text_margin),
        );
        config.set("InputPanel/Highlight/Image", "highlight.png");
        config.set("Menu/Highlight/Image", "highlight.png");
        set_margins(config, "InputPanel/Highlight", "Margin", (l, t, r, b));
        set_margins(config, "Menu/Highlight", "Margin", (l, t, r, b));
        set_margins(config, "InputPanel", "TextMargin", (l, t + text_margin, r, b + text_margin));
        set_margins(config, "Menu", "TextMargin", (l, t, r, b));
        Ok(())
    }

    /// Optional icons: prev/next page, submenu arrow, radio, separator.
    fn generate_icons(
        &self,
        theme: &Theme,
        css: &str,
        config: &mut theme_conf::Config,
        files: &mut Vec<String>,
    ) -> Result<()> {
        let arrows = PlasmaSvg::load(theme, "widgets/arrows", css)?;
        if arrows.has_element("left-arrow") && arrows.has_element("right-arrow") {
            let prev = arrows.render_element("left-arrow", ICON_SIZE_SMALL_MEDIUM, ICON_SIZE_SMALL_MEDIUM)?;
            let next = arrows.render_element("right-arrow", ICON_SIZE_SMALL_MEDIUM, ICON_SIZE_SMALL_MEDIUM)?;
            write_png(&self.output_dir, "prev.png", &prev, files)?;
            write_png(&self.output_dir, "next.png", &next, files)?;
            config.set("InputPanel/PrevPage/Image", "prev.png");
            config.set("InputPanel/NextPage/Image", "next.png");
        }
        if arrows.has_element("right-arrow") {
            let arrow = arrows.render_element("right-arrow", ICON_SIZE_SMALL, ICON_SIZE_SMALL)?;
            write_png(&self.output_dir, "arrow.png", &arrow, files)?;
            config.set("Menu/SubMenu/Image", "arrow.png");
        }

        let checkmarks = PlasmaSvg::load(theme, "widgets/checkmarks", css)?;
        if checkmarks.has_element("radiobutton") {
            let radio = checkmarks.render_element("radiobutton", ICON_SIZE_SMALL, ICON_SIZE_SMALL)?;
            write_png(&self.output_dir, "radio.png", &radio, files)?;
            config.set("Menu/CheckBox/Image", "radio.png");
        }

        let line = PlasmaSvg::load(theme, "widgets/line", css)?;
        if line.has_element("horizontal-line") {
            let image = line.render_element_native("horizontal-line")?;
            write_png(&self.output_dir, "line.png", &image, files)?;
            config.set("Menu/Separator/Image", "line.png");
        }
        Ok(())
    }

    /// SVG-mode panel: emit `panel.svg` (base frame composed at the shadow
    /// offset, shadow frame over it) and optional `mask.svg`, plus the
    /// theme.conf entries.
    ///
    /// Frames are composed into a 200×200 vector canvas with per-slice scale
    /// transforms (see [`svg::PlasmaSvg::emit_composed`]), so the `Margin`
    /// values computed from the hint elements — identical to the PNG path —
    /// are expressed in the SVG's own coordinate units and fcitx5 slices the
    /// document exactly like it slices the reference PNGs.
    fn generate_panel_svg(
        &self,
        theme: &Theme,
        css: &str,
        config: &mut theme_conf::Config,
        files: &mut Vec<String>,
    ) -> Result<()> {
        let mut background = PlasmaSvg::load(theme, "dialogs/background", css)?;

        let has_shadow = background.has_element_prefix("shadow");
        let shadow_margins = if has_shadow {
            background.frame("shadow", FRAME_SIZE, FRAME_SIZE)?.margins()
        } else {
            (0.0, 0.0, 0.0, 0.0)
        };
        let (shadow_left, shadow_top, shadow_right, shadow_bottom) = shadow_margins;
        let bg_margins = background.frame("", FRAME_SIZE, FRAME_SIZE)?.margins();
        let (bg_left, bg_top, bg_right, bg_bottom) = bg_margins;
        // Same numbers as the PNG path: content margins measure from the
        // panel's outer edge (bg hint margin + shadow offset).
        let content_margins = (
            bg_left + shadow_left,
            bg_top + shadow_top,
            bg_right + shadow_right,
            bg_bottom + shadow_bottom,
        );

        // The bg frame is composed at the panel size minus the shadow, placed
        // at the shadow offset; the shadow frame covers the full panel.
        let bg_size = (
            FRAME_SIZE - (shadow_left + shadow_right) as u32,
            FRAME_SIZE - (shadow_top + shadow_bottom) as u32,
        );
        let mut frames: Vec<svg::ComposedFrame<'_>> = Vec::new();
        frames.push(("", bg_size, (shadow_left, shadow_top)));
        if has_shadow {
            frames.push(("shadow", (FRAME_SIZE, FRAME_SIZE), (0.0, 0.0)));
        }
        let svg_bytes = background.emit_composed(&frames, (FRAME_SIZE, FRAME_SIZE))?;
        write_svg(&self.output_dir, "panel.svg", &svg_bytes, files)?;

        set_margins(config, "InputPanel", "ContentMargin", content_margins);
        set_margins(config, "Menu", "ContentMargin", content_margins);
        set_margins(config, "InputPanel", "ShadowMargin", shadow_margins);
        config.set("InputPanel/Background/Image", "panel.svg");
        config.set("Menu/Background/Image", "panel.svg");
        set_margins(config, "InputPanel/Background", "Margin", content_margins);
        set_margins(config, "Menu/Background", "Margin", content_margins);

        if theme.blur_behind {
            if !background.has_element_prefix("mask") {
                return Err(Error::missing_element(
                    background.path().to_path_buf(),
                    "mask",
                ));
            }
            // The mask frame is composed like the PNG mask (shrunk by the
            // shadow and an extra 2, placed 1 px inside the shadow edge);
            // fcitx5 slices it with the background's Margin.
            let mask_size = (
                FRAME_SIZE - (shadow_left * 2.0).round() as u32 - 2,
                FRAME_SIZE - (shadow_top * 2.0).round() as u32 - 2,
            );
            let mask_frames = [("mask", mask_size, (shadow_left + 1.0, shadow_top + 1.0))];
            let mask_bytes = background.emit_composed(&mask_frames, (FRAME_SIZE, FRAME_SIZE))?;
            write_svg(&self.output_dir, "mask.svg", &mask_bytes, files)?;
            config.set("InputPanel/BlurMask", "mask.svg");
            config.set("InputPanel/EnableBlur", "True");
        }
        Ok(())
    }

    /// SVG-mode highlight: `widgets/viewitem` hover/selected prefix composed
    /// into `highlight.svg`, with the same hint-derived margin numbers as
    /// the PNG path.
    fn generate_highlight_svg(
        &self,
        theme: &Theme,
        css: &str,
        config: &mut theme_conf::Config,
        files: &mut Vec<String>,
    ) -> Result<()> {
        let mut viewitem = PlasmaSvg::load(theme, "widgets/viewitem", css)?;
        let prefix = if viewitem.has_element_prefix("hover") {
            "hover"
        } else if viewitem.has_element_prefix("selected") {
            "selected"
        } else {
            return Ok(());
        };
        let frames = [(prefix, (FRAME_SIZE, FRAME_SIZE), (0.0, 0.0))];
        let svg_bytes = viewitem.emit_composed(&frames, (FRAME_SIZE, FRAME_SIZE))?;
        write_svg(&self.output_dir, "highlight.svg", &svg_bytes, files)?;

        let (l, t, r, b) = viewitem.frame(prefix, FRAME_SIZE, FRAME_SIZE)?.margins();
        // Mirror main.cpp: margins floored at the text margin.
        let text_margin = self.text_margin();
        let (l, t, r, b) = (
            l.max(text_margin),
            t.max(text_margin),
            r.max(text_margin),
            b.max(text_margin),
        );
        config.set("InputPanel/Highlight/Image", "highlight.svg");
        config.set("Menu/Highlight/Image", "highlight.svg");
        set_margins(config, "InputPanel/Highlight", "Margin", (l, t, r, b));
        set_margins(config, "Menu/Highlight", "Margin", (l, t, r, b));
        set_margins(config, "InputPanel", "TextMargin", (l, t + text_margin, r, b + text_margin));
        set_margins(config, "Menu", "TextMargin", (l, t, r, b));
        Ok(())
    }

    /// SVG-mode icons: arrows and radio emitted as standalone SVGs sized at
    /// their on-screen size (fcitx5 renders action images at the SVG's
    /// intrinsic size, 1:1); the separator keeps its full native document.
    fn generate_icons_svg(
        &self,
        theme: &Theme,
        css: &str,
        config: &mut theme_conf::Config,
        files: &mut Vec<String>,
    ) -> Result<()> {
        let arrows = PlasmaSvg::load(theme, "widgets/arrows", css)?;
        if arrows.has_element("left-arrow") && arrows.has_element("right-arrow") {
            let prev = arrows.emit_element("left-arrow", ICON_SIZE_SMALL_MEDIUM, ICON_SIZE_SMALL_MEDIUM)?;
            write_svg(&self.output_dir, "prev.svg", &prev, files)?;
            let next = arrows.emit_element("right-arrow", ICON_SIZE_SMALL_MEDIUM, ICON_SIZE_SMALL_MEDIUM)?;
            write_svg(&self.output_dir, "next.svg", &next, files)?;
            config.set("InputPanel/PrevPage/Image", "prev.svg");
            config.set("InputPanel/NextPage/Image", "next.svg");
        }
        if arrows.has_element("right-arrow") {
            let arrow = arrows.emit_element("right-arrow", ICON_SIZE_SMALL, ICON_SIZE_SMALL)?;
            write_svg(&self.output_dir, "arrow.svg", &arrow, files)?;
            config.set("Menu/SubMenu/Image", "arrow.svg");
        }

        let checkmarks = PlasmaSvg::load(theme, "widgets/checkmarks", css)?;
        if checkmarks.has_element("radiobutton") {
            let radio = checkmarks.emit_element("radiobutton", ICON_SIZE_SMALL, ICON_SIZE_SMALL)?;
            write_svg(&self.output_dir, "radio.svg", &radio, files)?;
            config.set("Menu/CheckBox/Image", "radio.svg");
        }

        let line = PlasmaSvg::load(theme, "widgets/line", css)?;
        if line.has_element("horizontal-line") {
            // The separator keeps the whole (already color-injected) native
            // document so its dashed pattern matches the reference PNG.
            write_svg(&self.output_dir, "line.svg", line.document(), files)?;
            config.set("Menu/Separator/Image", "line.svg");
        }
        Ok(())
    }

    /// `textMargin` = `smallSpacing / 2` with `smallSpacing = max(2, grid/4)`.
    #[must_use]
    fn text_margin(&self) -> f32 {
        let small_spacing = (self.grid_unit / 4).max(2);
        small_spacing as f32 / 2.0
    }
}

/// The blur mask: alpha mask of the frame painted black, mirroring
/// `QBitmap` semantics (1-bits paint as solid black).
fn generate_mask(
    background: &mut PlasmaSvg,
    shadow_left: f32,
    shadow_top: f32,
) -> Result<RgbaImage> {
    // The mask is built from the `mask-*` elements of the same SVG, with the
    // frame shrunk by the shadow margins and an extra 2 on each axis.
    let mask_w = FRAME_SIZE - (shadow_left * 2.0).round() as u32 - 2;
    let mask_h = FRAME_SIZE - (shadow_top * 2.0).round() as u32 - 2;
    let frame = background.frame("mask", mask_w, mask_h)?;
    let mask = frame.render()?;
    let mut out = RgbaImage::new(FRAME_SIZE, FRAME_SIZE);
    let offset_x = (shadow_left + 1.0) as u32;
    let offset_y = (shadow_top + 1.0) as u32;
    paste(&mut out, &mask, offset_x, offset_y);
    // QBitmap semantics: opaque → black.
    for pixel in out.pixels_mut() {
        if pixel.0[3] > 0 {
            pixel.0 = [0, 0, 0, 255];
        }
    }
    Ok(out)
}

/// Copy `src` onto `dst` at (x, y) with overwrite (Source) semantics.
fn paste(dst: &mut RgbaImage, src: &RgbaImage, x: u32, y: u32) {
    for (sy, row) in src.rows().enumerate() {
        let dy = y + sy as u32;
        if dy >= dst.height() {
            break;
        }
        for (sx, pixel) in row.enumerate() {
            let dx = x + sx as u32;
            if dx >= dst.width() {
                break;
            }
            dst.put_pixel(dx, dy, *pixel);
        }
    }
}

/// Blend `src` over `dst` at (x, y) with source-over semantics.
fn composite_over(dst: &mut RgbaImage, src: &RgbaImage, x: u32, y: u32) {
    for (sy, row) in src.rows().enumerate() {
        let dy = y + sy as u32;
        if dy >= dst.height() {
            break;
        }
        for (sx, pixel) in row.enumerate() {
            let dx = x + sx as u32;
            if dx >= dst.width() {
                break;
            }
            let s = pixel.0;
            if s[3] == 0 {
                continue;
            }
            let d = dst.get_pixel(dx, dy).0;
            let sa = u32::from(s[3]);
            let da = u32::from(d[3]);
            let oa = sa + da * (255 - sa) / 255;
            if oa == 0 {
                continue;
            }
            let out = [
                ((u32::from(s[0]) * sa + u32::from(d[0]) * da * (255 - sa) / 255) / oa) as u8,
                ((u32::from(s[1]) * sa + u32::from(d[1]) * da * (255 - sa) / 255) / oa) as u8,
                ((u32::from(s[2]) * sa + u32::from(d[2]) * da * (255 - sa) / 255) / oa) as u8,
                oa as u8,
            ];
            dst.put_pixel(dx, dy, image::Rgba(out));
        }
    }
}

fn set_margins(config: &mut theme_conf::Config, section: &str, key: &str, margins: (f32, f32, f32, f32)) {
    let (left, top, right, bottom) = margins;
    config.set(&format!("{section}/{key}/Left"), &format!("{}", left.round() as i64));
    config.set(&format!("{section}/{key}/Top"), &format!("{}", top.round() as i64));
    config.set(&format!("{section}/{key}/Right"), &format!("{}", right.round() as i64));
    config.set(&format!("{section}/{key}/Bottom"), &format!("{}", bottom.round() as i64));
}

fn write_png(
    dir: &Path,
    name: &str,
    image: &RgbaImage,
    files: &mut Vec<String>,
) -> Result<()> {
    let path = dir.join(name);
    image.save(&path).map_err(|e| {
        Error::other(format!("failed to save {name}: {e}", name = path.display()))
    })?;
    files.push(name.to_owned());
    Ok(())
}

fn write_svg(dir: &Path, name: &str, data: &[u8], files: &mut Vec<String>) -> Result<()> {
    let path = dir.join(name);
    std::fs::write(&path, data).map_err(|source| Error::io(path.clone(), source))?;
    files.push(name.to_owned());
    Ok(())
}
