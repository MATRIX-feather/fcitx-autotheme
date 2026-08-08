// SPDX-FileCopyrightText: 2026 Fcitx Breeze Theme Generator Authors
//
// SPDX-License-Identifier: GPL-2.0-or-later
//
//! KSvg-compatible `Svg` and `FrameSvg` classes.
//!
//! Faithfully replicates the geometry/margin logic of KSvg's `FrameSvg`
//! (`updateSizes`, `getMargins`, 9-patch `generateFrameBackground`,
//! `alphaMask`) and `Svg::pixmap` element rendering, as used by the original
//! generator.

use crate::render::{Pixmap, SvgDoc};
use std::path::Path;

/// KSvg `Svg` — an SVG document with element access and pixmap rendering.
pub struct Svg {
    doc: Option<SvgDoc>,
    multiple_images: bool,
}

impl Default for Svg {
    fn default() -> Self {
        Self::new()
    }
}

impl Svg {
    pub fn new() -> Self {
        Svg {
            doc: None,
            multiple_images: false,
        }
    }

    /// `setContainsMultipleImages` — controls pixmap sizing (see `pixmap`).
    pub fn set_contains_multiple_images(&mut self, v: bool) {
        self.multiple_images = v;
    }

    /// Load from a file, applying the color-scheme stylesheet.
    pub fn set_image_path(&mut self, path: &Path, stylesheet: &str) {
        self.doc = crate::render::load_svg_file(path, stylesheet).ok();
    }

    pub fn is_loaded(&self) -> bool {
        self.doc.is_some()
    }

    fn doc(&self) -> &SvgDoc {
        self.doc.as_ref().expect("svg not loaded")
    }

    fn doc_mut(&mut self) -> &mut SvgDoc {
        self.doc.as_mut().expect("svg not loaded")
    }

    /// `Svg::resize` — set target logical size.
    pub fn resize(&mut self, w: f32, h: f32) {
        self.doc_mut().resize(w, h);
    }

    pub fn has_element(&self, id: &str) -> bool {
        self.doc().has_element(id)
    }

    pub fn element_rect(&self, id: &str) -> Option<tiny_skia::Rect> {
        self.doc().element_rect(id)
    }

    pub fn element_size(&self, id: &str) -> (f32, f32) {
        self.doc().element_size(id).unwrap_or((0.0, 0.0))
    }

    pub fn doc_size(&self) -> (f32, f32) {
        self.doc().size()
    }

    /// `Svg::pixmap(elementId)`.
    ///
    /// With `multipleImages == true` (which FrameSvg always sets and the
    /// generator sets on the arrows/checkmarks/line SVGs) the pixmap is the
    /// resized `size()`; otherwise it is the element's own scaled rect.
    pub fn pixmap(&self, element_id: &str) -> Option<Pixmap> {
        let doc = self.doc();
        let target = if self.multiple_images {
            let (w, h) = doc.size();
            (w as u32, h as u32)
        } else {
            let (w, h) = doc.element_size(element_id)?;
            (w as u32, h as u32)
        };
        doc.render_element(element_id, target)
    }

    /// Render an element into a pixmap of the given size (used by FrameSvg).
    pub fn render_element(&self, id: &str, w: u32, h: u32) -> Option<Pixmap> {
        self.doc().render_element(id, (w, h))
    }
}

/// `FrameSvg::EnabledBorder` bit flags.
pub const BORDER_LEFT: u8 = 1 << 0;
pub const BORDER_RIGHT: u8 = 1 << 1;
pub const BORDER_TOP: u8 = 1 << 2;
pub const BORDER_BOTTOM: u8 = 1 << 3;
pub const ALL_BORDERS: u8 = BORDER_LEFT | BORDER_RIGHT | BORDER_TOP | BORDER_BOTTOM;

/// Per-frame geometry computed by `FrameSvgPrivate::updateSizes`.
#[derive(Debug, Clone, Default)]
struct FrameData {
    prefix: String,
    enabled_borders: u8,
    frame_size: (f32, f32),
    fixed_top_height: f32,
    fixed_bottom_height: f32,
    fixed_left_width: f32,
    fixed_right_width: f32,
    fixed_top_margin: f32,
    fixed_bottom_margin: f32,
    fixed_left_margin: f32,
    fixed_right_margin: f32,
    top_margin: f32,
    bottom_margin: f32,
    left_margin: f32,
    right_margin: f32,
    top_height: f32,
    bottom_height: f32,
    left_width: f32,
    right_width: f32,
    tile_center: bool,
    no_border_padding: bool,
    stretch_borders: bool,
    compose_over_border: bool,
}

/// KSvg `FrameSvg` — a 9-patch frame with hint-based margins.
pub struct FrameSvg {
    svg: Svg,
    prefix: String,
    requested_prefix: String,
    frame: FrameData,
    cached_background: Option<Pixmap>,
    frame_size_set: bool,
    sizes_dirty: bool,
}

impl Default for FrameSvg {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameSvg {
    pub fn new() -> Self {
        FrameSvg {
            svg: Svg::new(),
            prefix: String::new(),
            requested_prefix: String::new(),
            frame: FrameData {
                enabled_borders: ALL_BORDERS,
                ..Default::default()
            },
            cached_background: None,
            frame_size_set: false,
            sizes_dirty: true,
        }
    }

    pub fn svg_mut(&mut self) -> &mut Svg {
        &mut self.svg
    }

    pub fn set_image_path(&mut self, path: &Path, stylesheet: &str) {
        self.svg.set_contains_multiple_images(true);
        self.svg.set_image_path(path, stylesheet);
        self.prefix.clear();
        self.requested_prefix.clear();
        self.frame_size_set = false;
        self.frame = FrameData {
            enabled_borders: ALL_BORDERS,
            ..Default::default()
        };
        self.cached_background = None;
    }

    /// `FrameSvg::hasElementPrefix` — checks `<prefix>-center` (or `center`).
    pub fn has_element_prefix(&self, prefix: &str) -> bool {
        let id = if prefix.is_empty() {
            "center".to_string()
        } else if prefix.ends_with('-') {
            format!("{prefix}center")
        } else {
            format!("{prefix}-center")
        };
        self.svg.has_element(&id)
    }

    /// `FrameSvg::setElementPrefix`.
    pub fn set_element_prefix(&mut self, prefix: &str) {
        self.requested_prefix = prefix.to_string();
        let id = if prefix.is_empty() || prefix.ends_with('-') {
            format!("{prefix}center")
        } else {
            format!("{prefix}-center")
        };
        if prefix.is_empty() || !self.svg.has_element(&id) {
            self.prefix.clear();
        } else {
            self.prefix = if prefix.ends_with('-') {
                prefix.to_string()
            } else {
                format!("{prefix}-")
            };
        }
        self.frame = FrameData {
            enabled_borders: ALL_BORDERS,
            ..Default::default()
        };
        self.cached_background = None;
        self.sizes_dirty = true;
    }

    /// `FrameSvg::resizeFrame` — set the logical frame size.
    pub fn resize_frame(&mut self, w: f32, h: f32) {
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        self.frame_size_set = true;
        self.frame.frame_size = (w.round(), h.round());
        self.cached_background = None;
        self.sizes_dirty = true;
    }

    fn effective_frame_size(&self) -> (f32, f32) {
        if self.frame_size_set {
            self.frame.frame_size
        } else {
            self.svg.doc_size()
        }
    }

    /// `FrameSvg::getMargins`.
    pub fn get_margins(&mut self) -> (f32, f32, f32, f32) {
        self.ensure_sizes();
        if self.frame.no_border_padding {
            return (0.0, 0.0, 0.0, 0.0);
        }
        (
            self.frame.left_margin,
            self.frame.top_margin,
            self.frame.right_margin,
            self.frame.bottom_margin,
        )
    }

    /// Run `updateSizes` only when the frame geometry changed.
    fn ensure_sizes(&mut self) {
        if self.sizes_dirty {
            self.update_sizes();
            self.sizes_dirty = false;
        }
    }

    /// Run `updateSizes` (the hint-element margin computation).
    pub fn update_sizes(&mut self) {
        let f = &mut self.frame;
        let svg = &self.svg;
        let cn = |suffix: &str| format!("{}{}", self.prefix, suffix);

        f.fixed_top_height = svg.element_size(&cn("top")).1;
        f.fixed_top_margin = svg
            .element_rect(&cn("hint-top-margin"))
            .map(|r| r.height())
            .unwrap_or(f.fixed_top_height);
        f.fixed_bottom_height = svg.element_size(&cn("bottom")).1;
        f.fixed_bottom_margin = svg
            .element_rect(&cn("hint-bottom-margin"))
            .map(|r| r.height())
            .unwrap_or(f.fixed_bottom_height);
        f.fixed_left_width = svg.element_size(&cn("left")).0;
        f.fixed_left_margin = svg
            .element_rect(&cn("hint-left-margin"))
            .map(|r| r.width())
            .unwrap_or(f.fixed_left_width);
        f.fixed_right_width = svg.element_size(&cn("right")).0;
        f.fixed_right_margin = svg
            .element_rect(&cn("hint-right-margin"))
            .map(|r| r.width())
            .unwrap_or(f.fixed_right_width);

        let borders = f.enabled_borders;
        if borders & BORDER_TOP != 0 {
            f.top_margin = f.fixed_top_margin;
            f.top_height = f.fixed_top_height;
        } else {
            f.top_margin = 0.0;
            f.top_height = 0.0;
        }
        if borders & BORDER_BOTTOM != 0 {
            f.bottom_margin = f.fixed_bottom_margin;
            f.bottom_height = f.fixed_bottom_height;
        } else {
            f.bottom_margin = 0.0;
            f.bottom_height = 0.0;
        }
        if borders & BORDER_LEFT != 0 {
            f.left_margin = f.fixed_left_margin;
            f.left_width = f.fixed_left_width;
        } else {
            f.left_margin = 0.0;
            f.left_width = 0.0;
        }
        if borders & BORDER_RIGHT != 0 {
            f.right_margin = f.fixed_right_margin;
            f.right_width = f.fixed_right_width;
        } else {
            f.right_margin = 0.0;
            f.right_width = 0.0;
        }

        f.tile_center = svg.has_element("hint-tile-center") || svg.has_element(&cn("hint-tile-center"));
        f.no_border_padding =
            svg.has_element("hint-no-border-padding") || svg.has_element(&cn("hint-no-border-padding"));
        f.stretch_borders =
            svg.has_element("hint-stretch-borders") || svg.has_element(&cn("hint-stretch-borders"));
        f.compose_over_border = svg.has_element(&cn("hint-compose-over-border"))
            && svg.has_element(&format!("mask-{}center", f.prefix));
    }

    /// `FrameSvg::framePixmap()` — the pre-rendered 9-patch background.
    pub fn frame_pixmap(&mut self) -> Option<Pixmap> {
        if self.cached_background.is_none() {
            self.ensure_sizes();
            self.cached_background = self.generate_frame_background();
        }
        self.cached_background.clone()
    }

    /// `FrameSvg::alphaMask()` — the alpha-channel mask of the frame.
    pub fn alpha_mask(&mut self) -> Option<Pixmap> {
        if !self.svg.has_element(&format!("mask-{}center", self.prefix)) {
            return self.frame_pixmap();
        }
        let saved_prefix = self.prefix.clone();
        let saved_size = self.frame.frame_size;
        self.prefix = format!("mask-{saved_prefix}");
        self.frame = FrameData {
            enabled_borders: ALL_BORDERS,
            frame_size: saved_size,
            ..Default::default()
        };
        self.cached_background = None;
        self.sizes_dirty = true;
        let result = self.frame_pixmap();
        self.prefix = saved_prefix;
        self.frame = FrameData {
            enabled_borders: ALL_BORDERS,
            ..Default::default()
        };
        self.cached_background = None;
        self.sizes_dirty = true;
        result
    }

    /// The 9-patch compositing, replicating `generateFrameBackground`.
    fn generate_frame_background(&mut self) -> Option<Pixmap> {
        let size = self.effective_frame_size();
        let (sw, sh) = (size.0.ceil() as u32, size.1.ceil() as u32);
        let mut canvas = Pixmap::new(sw, sh);

        // Snapshot geometry to avoid borrow conflicts with &mut self calls.
        let f = self.frame.clone();
        let content_size = (
            size.0 - f.left_width - f.right_width,
            size.1 - f.top_height - f.bottom_height,
        );
        if content_size.0 < 0.0 || content_size.1 < 0.0 {
            return Some(canvas);
        }
        let mut content_rect = tiny_skia::Rect::from_xywh(0.0, 0.0, content_size.0, content_size.1)?;
        let has_left = self.svg.has_element(&format!("{}left", self.prefix));
        let has_top = self.svg.has_element(&format!("{}top", self.prefix));
        if f.enabled_borders & BORDER_LEFT != 0 && has_left {
            content_rect = tiny_skia::Rect::from_xywh(
                content_rect.x() + f.left_width,
                content_rect.y(),
                content_rect.width(),
                content_rect.height(),
            )
            .unwrap();
        }
        if f.enabled_borders & BORDER_TOP != 0 && has_top {
            content_rect = tiny_skia::Rect::from_xywh(
                content_rect.x(),
                content_rect.y() + f.top_height,
                content_rect.width(),
                content_rect.height(),
            )
            .unwrap();
        }

        self.paint_center(&mut canvas, &content_rect);
        self.paint_corner(&mut canvas, &content_rect, size, BORDER_LEFT | BORDER_TOP, "topleft");
        self.paint_corner(
            &mut canvas,
            &content_rect,
            size,
            BORDER_RIGHT | BORDER_TOP,
            "topright",
        );
        self.paint_corner(
            &mut canvas,
            &content_rect,
            size,
            BORDER_LEFT | BORDER_BOTTOM,
            "bottomleft",
        );
        self.paint_corner(
            &mut canvas,
            &content_rect,
            size,
            BORDER_RIGHT | BORDER_BOTTOM,
            "bottomright",
        );

        let left_height = self.svg.element_size(&format!("{}left", self.prefix)).1;
        self.paint_border(
            &mut canvas,
            &content_rect,
            size,
            BORDER_LEFT,
            (f.left_width, left_height),
        );
        let right_height = self.svg.element_size(&format!("{}right", self.prefix)).1;
        self.paint_border(
            &mut canvas,
            &content_rect,
            size,
            BORDER_RIGHT,
            (f.right_width, right_height),
        );
        let top_width = self.svg.element_size(&format!("{}top", self.prefix)).0;
        self.paint_border(
            &mut canvas,
            &content_rect,
            size,
            BORDER_TOP,
            (top_width, f.top_height),
        );
        let bottom_width = self.svg.element_size(&format!("{}bottom", self.prefix)).0;
        self.paint_border(
            &mut canvas,
            &content_rect,
            size,
            BORDER_BOTTOM,
            (bottom_width, f.bottom_height),
        );

        Some(canvas)
    }

    fn paint_center(&mut self, canvas: &mut Pixmap, content_rect: &tiny_skia::Rect) {
        let f = self.frame.clone();
        if content_rect.width() <= 0.0 || content_rect.height() <= 0.0 {
            return;
        }
        let center_id = format!("{}center", self.prefix);
        if f.tile_center {
            let (tw, th) = self.svg.element_size(&center_id);
            if let Some(tile) = self.svg.render_element(&center_id, tw as u32, th as u32) {
                tile_rect(
                    canvas,
                    content_rect.x() as i32,
                    content_rect.y() as i32,
                    content_rect.width() as i32,
                    content_rect.height() as i32,
                    &tile,
                );
            }
        } else {
            let w = content_rect.width().max(1.0) as u32;
            let h = content_rect.height().max(1.0) as u32;
            if let Some(pm) = self.svg.render_element(&center_id, w, h) {
                canvas.draw_pixmap(content_rect.x() as i32, content_rect.y() as i32, &pm);
            }
        }
    }

    fn paint_corner(
        &mut self,
        canvas: &mut Pixmap,
        content_rect: &tiny_skia::Rect,
        size: (f32, f32),
        border: u8,
        name: &str,
    ) {
        let f = &self.frame;
        if (f.enabled_borders & border) != border {
            return;
        }
        let corner_id = format!("{}{}", self.prefix, name);
        if !self.svg.has_element(&corner_id) {
            return;
        }
        let r = section_rect(border, content_rect, size);
        let x = r.x().round() as i32;
        let y = r.y().round() as i32;
        let w = r.width().ceil().max(1.0) as u32;
        let h = r.height().ceil().max(1.0) as u32;
        if let Some(pm) = self.svg.render_element(&corner_id, w, h) {
            canvas.draw_pixmap(x, y, &pm);
        }
    }

    fn paint_border(
        &mut self,
        canvas: &mut Pixmap,
        content_rect: &tiny_skia::Rect,
        size: (f32, f32),
        border: u8,
        el_size: (f32, f32),
    ) {
        let f = &self.frame;
        let side = match border {
            BORDER_LEFT => "left",
            BORDER_RIGHT => "right",
            BORDER_TOP => "top",
            _ => "bottom",
        };
        let side_id = format!("{}{}", self.prefix, side);
        if f.enabled_borders & border == 0
            || !self.svg.has_element(&side_id)
            || el_size.0 <= 0.0
            || el_size.1 <= 0.0
        {
            return;
        }
        let r = section_rect(border, content_rect, size);
        if f.stretch_borders {
            let w = r.width().ceil().max(1.0) as u32;
            let h = r.height().ceil().max(1.0) as u32;
            if let Some(pm) = self.svg.render_element(&side_id, w, h) {
                canvas.draw_pixmap(r.x().round() as i32, r.y().round() as i32, &pm);
            }
        } else {
            let w = el_size.0.ceil().max(1.0) as u32;
            let h = el_size.1.ceil().max(1.0) as u32;
            if let Some(tile) = self.svg.render_element(&side_id, w, h) {
                tile_rect(
                    canvas,
                    r.x().round() as i32,
                    r.y().round() as i32,
                    r.width().ceil() as i32,
                    r.height().ceil() as i32,
                    &tile,
                );
            }
        }
    }
}

/// Tile `tile` over the rect (x, y, w, h), replicating `drawTiledPixmap`.
fn tile_rect(canvas: &mut Pixmap, x: i32, y: i32, w: i32, h: i32, tile: &Pixmap) {
    if w <= 0 || h <= 0 || tile.width == 0 || tile.height == 0 {
        return;
    }
    for ty in 0..h {
        let sy = ((ty % tile.height as i32) + tile.height as i32) % tile.height as i32;
        for tx in 0..w {
            let sx = ((tx % tile.width as i32) + tile.width as i32) % tile.width as i32;
            let px = &tile.data[((sy * tile.width as i32 + sx) * 4) as usize..];
            let dx = x + tx;
            let dy = y + ty;
            if dx < 0 || dy < 0 || dx >= canvas.width as i32 || dy >= canvas.height as i32 {
                continue;
            }
            blend_over_pixel(&mut canvas.data[((dy * canvas.width as i32 + dx) * 4) as usize..], px);
        }
    }
}

fn blend_over_pixel(dst: &mut [u8], src: &[u8]) {
    let sa = src[3] as u32;
    if sa == 0 {
        return;
    }
    let da = dst[3] as u32;
    let out_a = sa + da * (255 - sa) / 255;
    if out_a == 0 {
        dst[..4].fill(0);
        return;
    }
    for i in 0..3 {
        let s = src[i] as u32;
        let d = dst[i] as u32;
        dst[i] = ((s * sa + d * da * (255 - sa) / 255) / out_a) as u8;
    }
    dst[3] = out_a as u8;
}

/// `FrameSvgHelpers::sectionRect` (KF6 `QRectF`, exclusive right/bottom edges).
fn section_rect(borders: u8, content: &tiny_skia::Rect, full: (f32, f32)) -> tiny_skia::Rect {
    let (fw, fh) = full;
    let (x0, y0, x1, y1) = (content.x(), content.y(), content.right(), content.bottom());
    let (w, h) = (content.width(), content.height());
    match borders {
        b if b == BORDER_TOP => tiny_skia::Rect::from_xywh(x0, 0.0, w, y0).unwrap(),
        b if b == BORDER_BOTTOM => tiny_skia::Rect::from_xywh(x0, y1, w, fh - y1).unwrap(),
        b if b == BORDER_LEFT => tiny_skia::Rect::from_xywh(0.0, y0, x0, h).unwrap(),
        b if b == BORDER_RIGHT => tiny_skia::Rect::from_xywh(x1, y0, fw - x1, h).unwrap(),
        b if b == BORDER_TOP | BORDER_LEFT => tiny_skia::Rect::from_xywh(0.0, 0.0, x0, y0).unwrap(),
        b if b == BORDER_TOP | BORDER_RIGHT => tiny_skia::Rect::from_xywh(x1, 0.0, fw - x1, y0).unwrap(),
        b if b == BORDER_BOTTOM | BORDER_LEFT => {
            tiny_skia::Rect::from_xywh(0.0, y1, x0, fh - y1).unwrap()
        }
        b if b == BORDER_BOTTOM | BORDER_RIGHT => {
            tiny_skia::Rect::from_xywh(x1, y1, fw - x1, fh - y1).unwrap()
        }
        _ => *content,
    }
}
