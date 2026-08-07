//! KDE color scheme parsing and role resolution.
//!
//! Reimplements the parts of `KColorScheme` / `Plasma::Theme` that the
//! fcitx5-plasma-theme-generator relies on:
//!
//! - Parsing KDE color scheme data from either the Plasma 6 `kdeglobals`
//!   format (`[Colors:Window] BackgroundNormal=32,35,38`) or the legacy
//!   `.colors` format (`[Colors:Window] Background=#202326`).
//! - Resolving named CSS colors (`.ColorScheme-*`) exactly like
//!   `KSvg::ImageSetPrivate::namedColor` does (Window color set, normal
//!   status), and the theme.conf colors exactly like `Plasma::Theme::color`.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::Error;

/// An RGBA color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    /// Red channel.
    pub r: u8,
    /// Green channel.
    pub g: u8,
    /// Blue channel.
    pub b: u8,
    /// Alpha channel.
    pub a: u8,
}

impl Color {
    /// Build an opaque color.
    #[must_use]
    pub const fn opaque(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// fcitx `Color::toString()`: `#rrggbb` when opaque, `#rrggbbaa` otherwise.
    #[must_use]
    pub fn to_fcitx_string(self) -> String {
        if self.a == 255 {
            format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
        } else {
            format!(
                "#{:02x}{:02x}{:02x}{:02x}",
                self.r, self.g, self.b, self.a
            )
        }
    }

    /// `QColor::name()`-style: `#rrggbb`.
    #[must_use]
    pub fn to_css_string(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

/// A KDE color scheme group (`[Colors:Window]` etc.).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColorGroup {
    /// `[Colors:Window]`
    Window,
    /// `[Colors:View]`
    View,
    /// `[Colors:Button]`
    Button,
    /// `[Colors:Selection]`
    Selection,
    /// `[Colors:Tooltip]`
    Tooltip,
    /// `[Colors:Complementary]`
    Complementary,
    /// `[Colors:Header]`
    Header,
}

impl ColorGroup {
    /// Parse a group name, ignoring suffix qualifiers like `[Inactive]`.
    fn parse(name: &str) -> Option<Self> {
        let base = name.split('[').next().unwrap_or_default().trim();
        match base {
            "Colors:Window" => Some(Self::Window),
            "Colors:View" => Some(Self::View),
            "Colors:Button" => Some(Self::Button),
            "Colors:Selection" => Some(Self::Selection),
            "Colors:Tooltip" => Some(Self::Tooltip),
            "Colors:Complementary" => Some(Self::Complementary),
            "Colors:Header" => Some(Self::Header),
            _ => None,
        }
    }

    /// `KColorScheme` fallback order when a key is missing from a color set.
    ///
    /// Each role resolves from its own color set first — `KColorScheme` reads
    /// `[Colors:Button]` for the Button set, `[Colors:View]` for the View
    /// set, and so on — before falling back to the sets `KColorScheme` uses
    /// to supply defaults.
    const fn fallback_chain(self) -> &'static [Self] {
        match self {
            Self::Window => &[Self::Window, Self::View, Self::Selection],
            Self::View => &[Self::View, Self::Window, Self::Selection],
            Self::Button => &[Self::Button, Self::Window, Self::View, Self::Selection],
            Self::Tooltip => &[Self::Tooltip, Self::Window, Self::View, Self::Selection],
            Self::Complementary => {
                &[Self::Complementary, Self::Window, Self::View, Self::Selection]
            }
            Self::Header => &[Self::Header, Self::Window, Self::View, Self::Selection],
            Self::Selection => &[Self::Selection, Self::View, Self::Window],
        }
    }
}

/// A color key inside a group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColorKey {
    /// `Background` / `BackgroundNormal`
    Background,
    /// `Foreground` / `ForegroundNormal`
    Foreground,
    /// `DecorationFocus`
    DecorationFocus,
    /// `DecorationHover`
    DecorationHover,
    /// `Highlight` (View group only)
    Highlight,
    /// `HighlightedText` (View group only)
    HighlightedText,
    /// `Link` / `ForegroundLink`
    Link,
    /// `Visited` / `ForegroundVisited`
    Visited,
    /// `PositiveText` / `ForegroundPositive`
    PositiveText,
    /// `NeutralText` / `ForegroundNeutral`
    NeutralText,
    /// `NegativeText` / `ForegroundNegative`
    NegativeText,
    /// `Frame`
    Frame,
}

/// Convert sRGB channels to HSL, each in `[0, 1]`.
fn rgb_to_hsl((r, g, b): (u8, u8, u8)) -> (f32, f32, f32) {
    let r = f32::from(r) / 255.0;
    let g = f32::from(g) / 255.0;
    let b = f32::from(b) / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let lightness = (max + min) / 2.0;
    if max == min {
        return (0.0, 0.0, lightness);
    }
    let delta = max - min;
    let saturation = if lightness > 0.5 {
        delta / (2.0 - max - min)
    } else {
        delta / (max + min)
    };
    let hue = if max == r {
        (g - b) / delta + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    };
    (hue / 6.0, saturation, lightness)
}

/// One sixth of the HSL hue-to-rgb interpolation.
fn hue_to_rgb(p: f32, q: f32, mut t: f32) -> f32 {
    if t < 0.0 {
        t += 1.0;
    }
    if t > 1.0 {
        t -= 1.0;
    }
    if t < 1.0 / 6.0 {
        p + (q - p) * 6.0 * t
    } else if t < 0.5 {
        q
    } else if t < 2.0 / 3.0 {
        p + (q - p) * (2.0 / 3.0 - t) * 6.0
    } else {
        p
    }
}

/// Convert HSL (each in `[0, 1]`) back to sRGB channels.
fn hsl_to_rgb((hue, saturation, lightness): (f32, f32, f32)) -> (u8, u8, u8) {
    if saturation <= 0.0 {
        let value = (lightness * 255.0).round().clamp(0.0, 255.0) as u8;
        return (value, value, value);
    }
    let q = if lightness < 0.5 {
        lightness * (1.0 + saturation)
    } else {
        lightness + saturation - lightness * saturation
    };
    let p = 2.0 * lightness - q;
    let channel = |t: f32| (hue_to_rgb(p, q, t) * 255.0).round().clamp(0.0, 255.0) as u8;
    (channel(hue + 1.0 / 3.0), channel(hue), channel(hue - 1.0 / 3.0))
}

/// Parse a color value: `#rrggbb`, `#rrggbbaa`, or `r,g,b`.
fn parse_color(path: &Path, key: &str, value: &str) -> Result<Color, Error> {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix('#') {
        let hex = hex.trim();
        match hex.len() {
            6 => {
                let rgb = u32::from_str_radix(hex, 16)
                    .map_err(|_| Error::invalid_color(path, key, value))?;
                Ok(Color::opaque(
                    ((rgb >> 16) & 0xff) as u8,
                    ((rgb >> 8) & 0xff) as u8,
                    (rgb & 0xff) as u8,
                ))
            }
            8 => {
                let rgba = u32::from_str_radix(hex, 16)
                    .map_err(|_| Error::invalid_color(path, key, value))?;
                Ok(Color {
                    r: ((rgba >> 24) & 0xff) as u8,
                    g: ((rgba >> 16) & 0xff) as u8,
                    b: ((rgba >> 8) & 0xff) as u8,
                    a: (rgba & 0xff) as u8,
                })
            }
            _ => Err(Error::invalid_color(path, key, value)),
        }
    } else {
        let mut parts = value.split(',').map(str::trim);
        let r = parts.next().map(str::parse::<u8>);
        let g = parts.next().map(str::parse::<u8>);
        let b = parts.next().map(str::parse::<u8>);
        match (r, g, b) {
            (Some(Ok(r)), Some(Ok(g)), Some(Ok(b))) if parts.next().is_none() => {
                Ok(Color::opaque(r, g, b))
            }
            _ => Err(Error::invalid_color(path, key, value)),
        }
    }
}

/// Map a raw key from either format onto a [`ColorKey`], ignoring unrelated keys.
fn normalize_key(key: &str) -> Option<ColorKey> {
    match key.trim() {
        "Background" | "BackgroundNormal" => Some(ColorKey::Background),
        "Foreground" | "ForegroundNormal" => Some(ColorKey::Foreground),
        "DecorationFocus" => Some(ColorKey::DecorationFocus),
        "DecorationHover" => Some(ColorKey::DecorationHover),
        "Highlight" => Some(ColorKey::Highlight),
        "HighlightedText" => Some(ColorKey::HighlightedText),
        "Link" | "ForegroundLink" => Some(ColorKey::Link),
        "Visited" | "ForegroundVisited" => Some(ColorKey::Visited),
        "PositiveText" | "ForegroundPositive" => Some(ColorKey::PositiveText),
        "NeutralText" | "ForegroundNeutral" => Some(ColorKey::NeutralText),
        "NegativeText" | "ForegroundNegative" => Some(ColorKey::NegativeText),
        "Frame" => Some(ColorKey::Frame),
        _ => None,
    }
}

/// A parsed KDE color scheme.
#[derive(Debug, Default, Clone)]
pub struct ColorScheme {
    groups: HashMap<ColorGroup, HashMap<ColorKey, Color>>,
}

impl ColorScheme {
    /// Parse from a `.colors` file or `kdeglobals` (both formats accepted).
    pub fn from_file(path: &Path) -> Result<Self, Error> {
        let text = fs::read_to_string(path)
            .map_err(|source| Error::io(path.to_path_buf(), source))?;
        Self::parse(&text, path)
    }

    /// Parse `KConfig` text.
    fn parse(text: &str, path: &Path) -> Result<Self, Error> {
        let mut groups: HashMap<ColorGroup, HashMap<ColorKey, Color>> = HashMap::new();
        let mut current: Option<ColorGroup> = None;
        for raw_line in text.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                current = ColorGroup::parse(&line[1..line.len() - 1]);
                continue;
            }
            let Some(group) = current else {
                continue;
            };
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let Some(key) = normalize_key(key) else {
                continue;
            };
            let value = value.trim().trim_matches('"');
            if value.is_empty() {
                continue;
            }
            let color = parse_color(path, key_name(key), value)?;
            groups.entry(group).or_default().insert(key, color);
        }
        Ok(Self { groups })
    }

    /// Look up a color with the `KColorScheme` fallback chain.
    fn get(&self, group: ColorGroup, key: ColorKey) -> Option<Color> {
        for fallback in group.fallback_chain() {
            if let Some(color) = self.groups.get(fallback).and_then(|g| g.get(&key)) {
                return Some(*color);
            }
        }
        None
    }

    /// Resolve a `KSvg` `StyleSheetColor` like
    /// `ImageSetPrivate::namedColor` (Window color set, normal status).
    #[must_use]
    #[allow(clippy::too_many_lines, reason = "flat role-to-color mapping table")]
    pub fn named_color(&self, name: StyleSheetColor) -> Color {
        match name {
            StyleSheetColor::Text => self
                .get(ColorGroup::Window, ColorKey::Foreground)
                .unwrap_or(Color::opaque(0, 0, 0)),
            StyleSheetColor::Background => self
                .get(ColorGroup::Window, ColorKey::Background)
                .unwrap_or(Color::opaque(255, 255, 255)),
            StyleSheetColor::Highlight => self
                .get(ColorGroup::Selection, ColorKey::Background)
                .unwrap_or(Color::opaque(0, 0, 0)),
            StyleSheetColor::HighlightedText => self
                .get(ColorGroup::Selection, ColorKey::Foreground)
                .unwrap_or(Color::opaque(255, 255, 255)),
            StyleSheetColor::ViewText => self
                .get(ColorGroup::View, ColorKey::Foreground)
                .unwrap_or(Color::opaque(0, 0, 0)),
            StyleSheetColor::ViewBackground => self
                .get(ColorGroup::View, ColorKey::Background)
                .unwrap_or(Color::opaque(255, 255, 255)),
            StyleSheetColor::ViewHover => self
                .get(ColorGroup::View, ColorKey::DecorationHover)
                .unwrap_or_else(|| self.named_color(StyleSheetColor::ViewText)),
            StyleSheetColor::ViewFocus => self
                .get(ColorGroup::View, ColorKey::DecorationFocus)
                .unwrap_or_else(|| self.named_color(StyleSheetColor::ViewHover)),
            StyleSheetColor::ButtonText => self
                .get(ColorGroup::Button, ColorKey::Foreground)
                .unwrap_or_else(|| self.named_color(StyleSheetColor::Text)),
            StyleSheetColor::ButtonBackground => self
                .get(ColorGroup::Button, ColorKey::Background)
                .unwrap_or_else(|| self.named_color(StyleSheetColor::Background)),
            StyleSheetColor::ButtonHover => self
                .get(ColorGroup::Button, ColorKey::DecorationHover)
                .unwrap_or_else(|| self.named_color(StyleSheetColor::ButtonText)),
            StyleSheetColor::ButtonFocus => self
                .get(ColorGroup::Button, ColorKey::DecorationFocus)
                .unwrap_or_else(|| self.named_color(StyleSheetColor::ButtonHover)),
            StyleSheetColor::PositiveText => self
                .get(ColorGroup::Window, ColorKey::PositiveText)
                .unwrap_or_else(|| self.named_color(StyleSheetColor::Text)),
            StyleSheetColor::NeutralText => self
                .get(ColorGroup::Window, ColorKey::NeutralText)
                .unwrap_or_else(|| self.named_color(StyleSheetColor::Text)),
            StyleSheetColor::NegativeText => self
                .get(ColorGroup::Window, ColorKey::NegativeText)
                .unwrap_or_else(|| self.named_color(StyleSheetColor::Text)),
            StyleSheetColor::TooltipText => self
                .get(ColorGroup::Tooltip, ColorKey::Foreground)
                .unwrap_or_else(|| self.named_color(StyleSheetColor::Text)),
            StyleSheetColor::TooltipBackground => self
                .get(ColorGroup::Tooltip, ColorKey::Background)
                .unwrap_or_else(|| self.named_color(StyleSheetColor::Background)),
            StyleSheetColor::TooltipHover => self
                .get(ColorGroup::Tooltip, ColorKey::DecorationHover)
                .unwrap_or_else(|| self.named_color(StyleSheetColor::TooltipText)),
            StyleSheetColor::TooltipFocus => self
                .get(ColorGroup::Tooltip, ColorKey::DecorationFocus)
                .unwrap_or_else(|| self.named_color(StyleSheetColor::TooltipHover)),
            StyleSheetColor::ComplementaryText => self
                .get(ColorGroup::Complementary, ColorKey::Foreground)
                .unwrap_or_else(|| self.named_color(StyleSheetColor::Text)),
            StyleSheetColor::ComplementaryBackground => self
                .get(ColorGroup::Complementary, ColorKey::Background)
                .unwrap_or_else(|| self.named_color(StyleSheetColor::Background)),
            StyleSheetColor::ComplementaryHover => self
                .get(ColorGroup::Complementary, ColorKey::DecorationHover)
                .unwrap_or_else(|| self.named_color(StyleSheetColor::ComplementaryText)),
            StyleSheetColor::ComplementaryFocus => self
                .get(ColorGroup::Complementary, ColorKey::DecorationFocus)
                .unwrap_or_else(|| self.named_color(StyleSheetColor::ComplementaryHover)),
            StyleSheetColor::HeaderText => self
                .get(ColorGroup::Header, ColorKey::Foreground)
                .unwrap_or_else(|| self.named_color(StyleSheetColor::Text)),
            StyleSheetColor::HeaderBackground => self
                .get(ColorGroup::Header, ColorKey::Background)
                .unwrap_or_else(|| self.named_color(StyleSheetColor::Background)),
            StyleSheetColor::HeaderHover => self
                .get(ColorGroup::Header, ColorKey::DecorationHover)
                .unwrap_or_else(|| self.named_color(StyleSheetColor::HeaderText)),
            StyleSheetColor::HeaderFocus => self
                .get(ColorGroup::Header, ColorKey::DecorationFocus)
                .unwrap_or_else(|| self.named_color(StyleSheetColor::HeaderHover)),
            StyleSheetColor::ButtonPositiveText => self
                .get(ColorGroup::Button, ColorKey::PositiveText)
                .unwrap_or_else(|| self.named_color(StyleSheetColor::ButtonText)),
            StyleSheetColor::ButtonNeutralText => self
                .get(ColorGroup::Button, ColorKey::NeutralText)
                .unwrap_or_else(|| self.named_color(StyleSheetColor::ButtonText)),
            StyleSheetColor::ButtonNegativeText => self
                .get(ColorGroup::Button, ColorKey::NegativeText)
                .unwrap_or_else(|| self.named_color(StyleSheetColor::ButtonText)),
            StyleSheetColor::ViewPositiveText => self
                .get(ColorGroup::View, ColorKey::PositiveText)
                .unwrap_or_else(|| self.named_color(StyleSheetColor::ViewText)),
            StyleSheetColor::ViewNeutralText => self
                .get(ColorGroup::View, ColorKey::NeutralText)
                .unwrap_or_else(|| self.named_color(StyleSheetColor::ViewText)),
            StyleSheetColor::ViewNegativeText => self
                .get(ColorGroup::View, ColorKey::NegativeText)
                .unwrap_or_else(|| self.named_color(StyleSheetColor::ViewText)),
            StyleSheetColor::TooltipPositiveText => self
                .get(ColorGroup::Tooltip, ColorKey::PositiveText)
                .unwrap_or_else(|| self.named_color(StyleSheetColor::TooltipText)),
            StyleSheetColor::TooltipNeutralText => self
                .get(ColorGroup::Tooltip, ColorKey::NeutralText)
                .unwrap_or_else(|| self.named_color(StyleSheetColor::TooltipText)),
            StyleSheetColor::TooltipNegativeText => self
                .get(ColorGroup::Tooltip, ColorKey::NegativeText)
                .unwrap_or_else(|| self.named_color(StyleSheetColor::TooltipText)),
            StyleSheetColor::ComplementaryPositiveText => self
                .get(ColorGroup::Complementary, ColorKey::PositiveText)
                .unwrap_or_else(|| self.named_color(StyleSheetColor::ComplementaryText)),
            StyleSheetColor::ComplementaryNeutralText => self
                .get(ColorGroup::Complementary, ColorKey::NeutralText)
                .unwrap_or_else(|| self.named_color(StyleSheetColor::ComplementaryText)),
            StyleSheetColor::ComplementaryNegativeText => self
                .get(ColorGroup::Complementary, ColorKey::NegativeText)
                .unwrap_or_else(|| self.named_color(StyleSheetColor::ComplementaryText)),
            StyleSheetColor::HeaderPositiveText => self
                .get(ColorGroup::Header, ColorKey::PositiveText)
                .unwrap_or_else(|| self.named_color(StyleSheetColor::HeaderText)),
            StyleSheetColor::HeaderNeutralText => self
                .get(ColorGroup::Header, ColorKey::NeutralText)
                .unwrap_or_else(|| self.named_color(StyleSheetColor::HeaderText)),
            StyleSheetColor::HeaderNegativeText => self
                .get(ColorGroup::Header, ColorKey::NegativeText)
                .unwrap_or_else(|| self.named_color(StyleSheetColor::HeaderText)),
            StyleSheetColor::HeaderHighlightedText
            | StyleSheetColor::ButtonHighlightedText
            | StyleSheetColor::ViewHighlightedText
            | StyleSheetColor::TooltipHighlightedText
            | StyleSheetColor::ComplementaryHighlightedText => self.named_color(StyleSheetColor::HighlightedText),
            StyleSheetColor::Frame => self
                .get(ColorGroup::Window, ColorKey::Frame)
                .or_else(|| self.get(ColorGroup::Window, ColorKey::Background))
                .unwrap_or(Color::opaque(255, 255, 255)),
        }
    }

    /// Resolve the theme.conf colors like `Plasma::Theme::color` does.
    #[must_use]
    pub fn theme_conf_colors(&self) -> ThemeConfColors {
        ThemeConfColors {
            normal: self.named_color(StyleSheetColor::Text),
            highlighted: self.named_color(StyleSheetColor::HighlightedText),
            highlight_background: self.named_color(StyleSheetColor::Highlight),
        }
    }

    /// Return a copy of this scheme with the accent-driven roles overridden.
    ///
    /// The desktop accent color drives the `Highlight` role (Selection
    /// `Background`, used by theme.conf and `.ColorScheme-Highlight`) and the
    /// decoration roles (`.ColorScheme-*Focus`/`*Hover`). KDE applies an
    /// accent by writing it into the `DecorationFocus`/`DecorationHover` keys
    /// of every color set (all `[Colors:*]` groups of kdeglobals), so we
    /// mirror that: Breeze's viewitem hover/selected frames and checkmarks
    /// radiobutton use `ColorScheme-ButtonFocus`, which must resolve to the
    /// accent through the Button set itself, not through a Window fallback.
    #[must_use]
    pub fn with_accent_color(mut self, accent: Color) -> Self {
        self.groups
            .entry(ColorGroup::Selection)
            .or_default()
            .insert(ColorKey::Background, accent);
        for group in [
            ColorGroup::Window,
            ColorGroup::View,
            ColorGroup::Button,
            ColorGroup::Tooltip,
            ColorGroup::Complementary,
            ColorGroup::Header,
            ColorGroup::Selection,
        ] {
            let colors = self.groups.entry(group).or_default();
            colors.insert(ColorKey::DecorationFocus, accent);
            colors.insert(ColorKey::DecorationHover, accent);
        }
        self
    }

    /// Deepen the highlight-driven roles by `percent` percent (0 = unchanged,
    /// 10 = 10% darker and 10% more saturated).
    ///
    /// The decoration roles (`.ColorScheme-*Focus`/`*Hover`, which color
    /// highlight.png and radio.png) and the `Highlight` role (theme.conf
    /// `HighlightBackgroundColor`) are darkened toward black and their
    /// saturation is boosted by the same percentage. Text, panel backgrounds
    /// and other roles are left untouched so contrast is kept.
    #[must_use]
    pub fn with_highlight_deepening(mut self, percent: u8) -> Self {
        if percent == 0 {
            return self;
        }
        let scale = 100 - u16::from(percent);
        let saturation_factor = 1.0 + f32::from(percent) / 100.0;
        let deepen = |color: &mut Color| {
            let (r, g, b) = (
                ((u16::from(color.r) * scale) / 100) as u8,
                ((u16::from(color.g) * scale) / 100) as u8,
                ((u16::from(color.b) * scale) / 100) as u8,
            );
            let (hue, saturation, lightness) = rgb_to_hsl((r, g, b));
            let (r, g, b) = hsl_to_rgb((hue, (saturation * saturation_factor).min(1.0), lightness));
            color.r = r;
            color.g = g;
            color.b = b;
        };
        // The Highlight role: `[Colors:Selection] Background`.
        if let Some(selection) = self.groups.get_mut(&ColorGroup::Selection)
            && let Some(background) = selection.get_mut(&ColorKey::Background)
        {
            deepen(background);
        }
        // Decoration roles in every color set.
        for group in self.groups.values_mut() {
            if let Some(focus) = group.get_mut(&ColorKey::DecorationFocus) {
                deepen(focus);
            }
            if let Some(hover) = group.get_mut(&ColorKey::DecorationHover) {
                deepen(hover);
            }
        }
        self
    }
}

/// The three colors written into `theme.conf`.
#[derive(Clone, Copy, Debug)]
pub struct ThemeConfColors {
    /// `NormalColor` / `HighlightCandidateColor`
    pub normal: Color,
    /// `HighlightColor`
    pub highlighted: Color,
    /// `HighlightBackgroundColor`
    pub highlight_background: Color,
}

/// A `KSvg` `StyleSheetColor` used to build the `current-color-scheme` CSS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StyleSheetColor {
    /// `.ColorScheme-Text`
    Text,
    /// `.ColorScheme-Background`
    Background,
    /// `.ColorScheme-Highlight`
    Highlight,
    /// `.ColorScheme-HighlightedText`
    HighlightedText,
    /// `.ColorScheme-PositiveText`
    PositiveText,
    /// `.ColorScheme-NeutralText`
    NeutralText,
    /// `.ColorScheme-NegativeText`
    NegativeText,
    /// `.ColorScheme-ButtonText`
    ButtonText,
    /// `.ColorScheme-ButtonBackground`
    ButtonBackground,
    /// `.ColorScheme-ButtonHover`
    ButtonHover,
    /// `.ColorScheme-ButtonFocus`
    ButtonFocus,
    /// `.ColorScheme-ButtonHighlightedText`
    ButtonHighlightedText,
    /// `.ColorScheme-ButtonPositiveText`
    ButtonPositiveText,
    /// `.ColorScheme-ButtonNeutralText`
    ButtonNeutralText,
    /// `.ColorScheme-ButtonNegativeText`
    ButtonNegativeText,
    /// `.ColorScheme-ViewText`
    ViewText,
    /// `.ColorScheme-ViewBackground`
    ViewBackground,
    /// `.ColorScheme-ViewHover`
    ViewHover,
    /// `.ColorScheme-ViewFocus`
    ViewFocus,
    /// `.ColorScheme-ViewHighlightedText`
    ViewHighlightedText,
    /// `.ColorScheme-ViewPositiveText`
    ViewPositiveText,
    /// `.ColorScheme-ViewNeutralText`
    ViewNeutralText,
    /// `.ColorScheme-ViewNegativeText`
    ViewNegativeText,
    /// `.ColorScheme-TooltipText`
    TooltipText,
    /// `.ColorScheme-TooltipBackground`
    TooltipBackground,
    /// `.ColorScheme-TooltipHover`
    TooltipHover,
    /// `.ColorScheme-TooltipFocus`
    TooltipFocus,
    /// `.ColorScheme-TooltipHighlightedText`
    TooltipHighlightedText,
    /// `.ColorScheme-TooltipPositiveText`
    TooltipPositiveText,
    /// `.ColorScheme-TooltipNeutralText`
    TooltipNeutralText,
    /// `.ColorScheme-TooltipNegativeText`
    TooltipNegativeText,
    /// `.ColorScheme-ComplementaryText`
    ComplementaryText,
    /// `.ColorScheme-ComplementaryBackground`
    ComplementaryBackground,
    /// `.ColorScheme-ComplementaryHover`
    ComplementaryHover,
    /// `.ColorScheme-ComplementaryFocus`
    ComplementaryFocus,
    /// `.ColorScheme-ComplementaryHighlightedText`
    ComplementaryHighlightedText,
    /// `.ColorScheme-ComplementaryPositiveText`
    ComplementaryPositiveText,
    /// `.ColorScheme-ComplementaryNeutralText`
    ComplementaryNeutralText,
    /// `.ColorScheme-ComplementaryNegativeText`
    ComplementaryNegativeText,
    /// `.ColorScheme-HeaderText`
    HeaderText,
    /// `.ColorScheme-HeaderBackground`
    HeaderBackground,
    /// `.ColorScheme-HeaderHover`
    HeaderHover,
    /// `.ColorScheme-HeaderFocus`
    HeaderFocus,
    /// `.ColorScheme-HeaderHighlightedText`
    HeaderHighlightedText,
    /// `.ColorScheme-HeaderPositiveText`
    HeaderPositiveText,
    /// `.ColorScheme-HeaderNeutralText`
    HeaderNeutralText,
    /// `.ColorScheme-HeaderNegativeText`
    HeaderNegativeText,
    /// `.ColorScheme-Frame`
    Frame,
}

impl StyleSheetColor {
    /// The CSS class name (without the `.ColorScheme-` prefix).
    #[must_use]
    pub const fn css_name(self) -> &'static str {
        match self {
            Self::Text => "Text",
            Self::Background => "Background",
            Self::Highlight => "Highlight",
            Self::HighlightedText => "HighlightedText",
            Self::PositiveText => "PositiveText",
            Self::NeutralText => "NeutralText",
            Self::NegativeText => "NegativeText",
            Self::ButtonText => "ButtonText",
            Self::ButtonBackground => "ButtonBackground",
            Self::ButtonHover => "ButtonHover",
            Self::ButtonFocus => "ButtonFocus",
            Self::ButtonHighlightedText => "ButtonHighlightedText",
            Self::ButtonPositiveText => "ButtonPositiveText",
            Self::ButtonNeutralText => "ButtonNeutralText",
            Self::ButtonNegativeText => "ButtonNegativeText",
            Self::ViewText => "ViewText",
            Self::ViewBackground => "ViewBackground",
            Self::ViewHover => "ViewHover",
            Self::ViewFocus => "ViewFocus",
            Self::ViewHighlightedText => "ViewHighlightedText",
            Self::ViewPositiveText => "ViewPositiveText",
            Self::ViewNeutralText => "ViewNeutralText",
            Self::ViewNegativeText => "ViewNegativeText",
            Self::TooltipText => "TooltipText",
            Self::TooltipBackground => "TooltipBackground",
            Self::TooltipHover => "TooltipHover",
            Self::TooltipFocus => "TooltipFocus",
            Self::TooltipHighlightedText => "TooltipHighlightedText",
            Self::TooltipPositiveText => "TooltipPositiveText",
            Self::TooltipNeutralText => "TooltipNeutralText",
            Self::TooltipNegativeText => "TooltipNegativeText",
            Self::ComplementaryText => "ComplementaryText",
            Self::ComplementaryBackground => "ComplementaryBackground",
            Self::ComplementaryHover => "ComplementaryHover",
            Self::ComplementaryFocus => "ComplementaryFocus",
            Self::ComplementaryHighlightedText => "ComplementaryHighlightedText",
            Self::ComplementaryPositiveText => "ComplementaryPositiveText",
            Self::ComplementaryNeutralText => "ComplementaryNeutralText",
            Self::ComplementaryNegativeText => "ComplementaryNegativeText",
            Self::HeaderText => "HeaderText",
            Self::HeaderBackground => "HeaderBackground",
            Self::HeaderHover => "HeaderHover",
            Self::HeaderFocus => "HeaderFocus",
            Self::HeaderHighlightedText => "HeaderHighlightedText",
            Self::HeaderPositiveText => "HeaderPositiveText",
            Self::HeaderNeutralText => "HeaderNeutralText",
            Self::HeaderNegativeText => "HeaderNegativeText",
            Self::Frame => "Frame",
        }
    }
}

/// All style sheet colors, mirroring the `namedColors` list in
/// `ImageSetPrivate::svgStyleSheet`.
const ALL_STYLE_SHEET_COLORS: &[StyleSheetColor] = &[
    StyleSheetColor::Text,
    StyleSheetColor::Background,
    StyleSheetColor::Highlight,
    StyleSheetColor::HighlightedText,
    StyleSheetColor::PositiveText,
    StyleSheetColor::NeutralText,
    StyleSheetColor::NegativeText,
    StyleSheetColor::ButtonText,
    StyleSheetColor::ButtonBackground,
    StyleSheetColor::ButtonHover,
    StyleSheetColor::ButtonFocus,
    StyleSheetColor::ButtonHighlightedText,
    StyleSheetColor::ButtonPositiveText,
    StyleSheetColor::ButtonNeutralText,
    StyleSheetColor::ButtonNegativeText,
    StyleSheetColor::ViewText,
    StyleSheetColor::ViewBackground,
    StyleSheetColor::ViewHover,
    StyleSheetColor::ViewFocus,
    StyleSheetColor::ViewHighlightedText,
    StyleSheetColor::ViewPositiveText,
    StyleSheetColor::ViewNeutralText,
    StyleSheetColor::ViewNegativeText,
    StyleSheetColor::TooltipText,
    StyleSheetColor::TooltipBackground,
    StyleSheetColor::TooltipHover,
    StyleSheetColor::TooltipFocus,
    StyleSheetColor::TooltipHighlightedText,
    StyleSheetColor::TooltipPositiveText,
    StyleSheetColor::TooltipNeutralText,
    StyleSheetColor::TooltipNegativeText,
    StyleSheetColor::ComplementaryText,
    StyleSheetColor::ComplementaryBackground,
    StyleSheetColor::ComplementaryHover,
    StyleSheetColor::ComplementaryFocus,
    StyleSheetColor::ComplementaryHighlightedText,
    StyleSheetColor::ComplementaryPositiveText,
    StyleSheetColor::ComplementaryNeutralText,
    StyleSheetColor::ComplementaryNegativeText,
    StyleSheetColor::HeaderText,
    StyleSheetColor::HeaderBackground,
    StyleSheetColor::HeaderHover,
    StyleSheetColor::HeaderFocus,
    StyleSheetColor::HeaderHighlightedText,
    StyleSheetColor::HeaderPositiveText,
    StyleSheetColor::HeaderNeutralText,
    StyleSheetColor::HeaderNegativeText,
    StyleSheetColor::Frame,
];

/// Build the `current-color-scheme` CSS block, mirroring
/// `ImageSetPrivate::svgStyleSheet`.
#[must_use]
pub fn style_sheet(scheme: &ColorScheme) -> String {
    use std::fmt::Write as _;
    let mut css = String::new();
    for color in ALL_STYLE_SHEET_COLORS {
        let _ = write!(
            css,
            ".ColorScheme-{}{{color:{};}}",
            color.css_name(),
            scheme.named_color(*color).to_css_string()
        );
    }
    css
}

/// Load the active color scheme.
///
/// Resolution order mirrors `KColorScheme`: if `kdeglobals` names a color
/// scheme (`[General] ColorScheme=...`) the matching `.colors` file is used;
/// otherwise the `[Colors:*]` groups of `kdeglobals` itself (Plasma 6
/// behavior) are used.
pub fn load_active_color_scheme() -> Result<ColorScheme, Error> {
    let kdeglobals = kdeglobals_path();
    let text = fs::read_to_string(&kdeglobals)
        .map_err(|source| Error::io(kdeglobals.clone(), source))?;
    let named = general_color_scheme_name(&text);
    if let Some(name) = named
        && let Some(path) = find_color_scheme_file(&name)
    {
        return ColorScheme::from_file(&path);
    }
    ColorScheme::parse(&text, &kdeglobals)
}

/// Load the color scheme named by the D-Bus signal, falling back to the
/// active scheme when `name` is absent or cannot be resolved on disk.
pub fn load_scheme_for_name(name: Option<&str>) -> Result<ColorScheme, Error> {
    name.and_then(find_color_scheme_file)
        .map_or_else(load_active_color_scheme, |path| ColorScheme::from_file(&path))
}

/// Path of the user's `kdeglobals` file.
#[must_use]
pub fn kdeglobals_path() -> PathBuf {
    let config = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    config.join("kdeglobals")
}

/// Extract `[General] ColorScheme=` from kdeglobals text.
fn general_color_scheme_name(text: &str) -> Option<String> {
    let mut in_general = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_general = line == "[General]";
            continue;
        }
        if in_general
            && let Some((key, value)) = line.split_once('=')
            && key.trim() == "ColorScheme"
        {
            let value = value.trim().trim_matches('"');
            if !value.is_empty() {
                return Some(value.to_owned());
            }
        }
    }
    None
}

/// Search the standard color-scheme directories for `<name>.colors`.
fn find_color_scheme_file(name: &str) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(data_home) = std::env::var_os("XDG_DATA_HOME") {
        candidates.push(PathBuf::from(data_home).join("color-schemes"));
    }
    if let Some(config) = dirs::config_dir() {
        candidates.push(config.join("color-schemes"));
    }
    if let Some(data_dirs) = std::env::var_os("XDG_DATA_DIRS") {
        for dir in std::env::split_paths(&data_dirs) {
            candidates.push(dir.join("color-schemes"));
        }
    }
    // Fallback for setups without XDG vars.
    if let Some(data) = dirs::data_dir() {
        candidates.push(data.join("color-schemes"));
    }
    for dir in candidates {
        let path = dir.join(format!("{name}.colors"));
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

const fn key_name(key: ColorKey) -> &'static str {
    match key {
        ColorKey::Background => "Background",
        ColorKey::Foreground => "Foreground",
        ColorKey::DecorationFocus => "DecorationFocus",
        ColorKey::DecorationHover => "DecorationHover",
        ColorKey::Highlight => "Highlight",
        ColorKey::HighlightedText => "HighlightedText",
        ColorKey::Link => "Link",
        ColorKey::Visited => "Visited",
        ColorKey::PositiveText => "PositiveText",
        ColorKey::NeutralText => "NeutralText",
        ColorKey::NegativeText => "NegativeText",
        ColorKey::Frame => "Frame",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_legacy_colors_file() {
        let text = "[Colors:Window]\nBackground=#112233\nForeground=#aabbcc\n";
        let scheme = ColorScheme::parse(text, Path::new("test")).expect("parse");
        assert_eq!(
            scheme.named_color(StyleSheetColor::Text),
            Color::opaque(0xaa, 0xbb, 0xcc)
        );
        assert_eq!(
            scheme.named_color(StyleSheetColor::Background),
            Color::opaque(0x11, 0x22, 0x33)
        );
    }

    #[test]
    fn parses_kdeglobals_new_format() {
        let text = "[Colors:Window]\nBackgroundNormal=17,34,51\nForegroundNormal=170,187,204\n[Colors:Selection]\nBackgroundNormal=255,136,0\n";
        let scheme = ColorScheme::parse(text, Path::new("test")).expect("parse");
        assert_eq!(
            scheme.named_color(StyleSheetColor::Text),
            Color::opaque(170, 187, 204)
        );
        // Highlight resolves to the Selection background.
        assert_eq!(
            scheme.named_color(StyleSheetColor::Highlight),
            Color::opaque(255, 136, 0)
        );
    }

    #[test]
    fn fcitx_color_string() {
        assert_eq!(Color::opaque(0xaa, 0xbb, 0xcc).to_fcitx_string(), "#aabbcc");
        assert_eq!(
            Color { r: 1, g: 2, b: 3, a: 128 }.to_fcitx_string(),
            "#01020380"
        );
    }

    #[test]
    fn extract_color_scheme_name() {
        assert_eq!(
            general_color_scheme_name("[General]\nColorScheme=Breeze Dark\n[KDE]\n"),
            Some("Breeze Dark".to_owned())
        );
        assert_eq!(general_color_scheme_name("[General]\nAccentColor=1,2,3\n"), None);
    }

    #[test]
    fn button_focus_uses_button_group_first() {
        // `KColorScheme` reads `[Colors:Button] DecorationFocus` for the
        // Button set; the Window value must not win when the groups differ
        // (regression: highlight.png/radio.png use `.ColorScheme-ButtonFocus`).
        let text = "[Colors:Window]\nDecorationFocus=#ffffff\nDecorationHover=#eeeeee\n[Colors:View]\nDecorationFocus=#dddddd\nDecorationHover=#cccccc\n[Colors:Button]\nDecorationFocus=#101010\nDecorationHover=#202020\n";
        let scheme = ColorScheme::parse(text, Path::new("test")).expect("parse");
        assert_eq!(
            scheme.named_color(StyleSheetColor::ButtonFocus),
            Color::opaque(0x10, 0x10, 0x10)
        );
        assert_eq!(
            scheme.named_color(StyleSheetColor::ButtonHover),
            Color::opaque(0x20, 0x20, 0x20)
        );
        // View roles resolve from the View set, not Window.
        assert_eq!(
            scheme.named_color(StyleSheetColor::ViewFocus),
            Color::opaque(0xdd, 0xdd, 0xdd)
        );
    }

    #[test]
    fn accent_reaches_all_decoration_groups() {
        // KDE writes the accent into every color set's DecorationFocus/Hover;
        // after that the Button set itself resolves ButtonFocus to the accent.
        let text = "[Colors:Window]\nDecorationFocus=#556677\n[Colors:Button]\nDecorationFocus=#667788\n[Colors:View]\nDecorationFocus=#445566\n";
        let accent = Color::opaque(0xe9, 0x3d, 0x58);
        let scheme = ColorScheme::parse(text, Path::new("test"))
            .expect("parse")
            .with_accent_color(accent);
        assert_eq!(scheme.named_color(StyleSheetColor::ButtonFocus), accent);
        assert_eq!(scheme.named_color(StyleSheetColor::ButtonHover), accent);
        assert_eq!(scheme.named_color(StyleSheetColor::ViewFocus), accent);
        assert_eq!(scheme.named_color(StyleSheetColor::TooltipFocus), accent);
        assert_eq!(scheme.named_color(StyleSheetColor::Highlight), accent);
    }

    #[test]
    fn highlight_deepening_scopes_decoration_and_highlight() {
        let text = "[Colors:Window]\nBackground=#ffffff\nDecorationFocus=#aaaaaa\nDecorationHover=#bbbbbb\n[Colors:Selection]\nBackground=#ff8800\nForeground=#ffffff\n";
        let scheme = ColorScheme::parse(text, Path::new("test")).expect("parse");
        let deep = scheme.clone().with_highlight_deepening(10);
        // Decoration roles (ButtonFocus colors highlight.png/radio.png) are
        // scaled toward black by 10%; gray and fully-saturated colors keep
        // their channel values after the saturation boost.
        assert_eq!(
            deep.named_color(StyleSheetColor::ButtonFocus),
            Color::opaque(0x99, 0x99, 0x99)
        );
        assert_eq!(
            deep.named_color(StyleSheetColor::ButtonHover),
            Color::opaque(0xa8, 0xa8, 0xa8)
        );
        // The Highlight role (Selection Background) is deepened too.
        assert_eq!(
            deep.named_color(StyleSheetColor::Highlight),
            Color::opaque(0xe5, 0x7a, 0x00)
        );
        // Everything else stays as-is: panel background, text and the
        // highlighted text keep their original colors for contrast.
        assert_eq!(
            deep.named_color(StyleSheetColor::Background),
            Color::opaque(0xff, 0xff, 0xff)
        );
        assert_eq!(
            deep.named_color(StyleSheetColor::HighlightedText),
            Color::opaque(0xff, 0xff, 0xff)
        );
        // Zero percent is a no-op.
        assert_eq!(scheme.named_color(StyleSheetColor::ButtonFocus), scheme.with_highlight_deepening(0).named_color(StyleSheetColor::ButtonFocus));
    }
}
