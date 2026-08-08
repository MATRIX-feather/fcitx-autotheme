// SPDX-FileCopyrightText: 2026 Fcitx Breeze Theme Generator Authors
//
// SPDX-License-Identifier: GPL-2.0-or-later
//
//! SVG loading and element rendering.
//!
//! Loads Plasma theme SVGs (`.svg` / `.svgz`), applies the color-scheme
//! stylesheet substitution (replicating KSvg's `SharedSvgRenderer::load`), and
//! renders single elements or 9-patch frame sections into RGBA pixmaps.

use std::path::Path;
use std::sync::Arc;

/// An RGBA pixmap (straight alpha; converted to premultiplied only at composite
/// time, mirroring how Qt stores `Format_ARGB32`).
#[derive(Debug, Clone)]
pub struct Pixmap {
    pub width: u32,
    pub height: u32,
    /// Non-premultiplied RGBA8, row-major.
    pub data: Vec<u8>,
}

impl Pixmap {
    pub fn new(width: u32, height: u32) -> Self {
        Pixmap {
            width,
            height,
            data: vec![0; (width * height * 4) as usize],
        }
    }

    pub fn fill_transparent(&mut self) {
        self.data.fill(0);
    }

    /// Blend `src` onto this pixmap at integer offset, source-over (straight alpha).
    pub fn draw_pixmap(&mut self, x: i32, y: i32, src: &Pixmap) {
        for sy in 0..src.height as i32 {
            let dy = y + sy;
            if dy < 0 || dy >= self.height as i32 {
                continue;
            }
            for sx in 0..src.width as i32 {
                let dx = x + sx;
                if dx < 0 || dx >= self.width as i32 {
                    continue;
                }
                blend_over(
                    &mut self.data[((dy * self.width as i32 + dx) * 4) as usize..],
                    &src.data[((sy * src.width as i32 + sx) * 4) as usize..],
                );
            }
        }
    }

    /// Convert to a `tiny_skia::Pixmap` (premultiplied) for rendering via resvg.
    pub fn to_tiny_skia(&self) -> tiny_skia::Pixmap {
        let mut p = tiny_skia::Pixmap::new(self.width, self.height).unwrap();
        for (dst, src) in p.data_mut().chunks_exact_mut(4).zip(self.data.chunks_exact(4)) {
            let a = src[3] as u32;
            dst[0] = (src[0] as u32 * a / 255) as u8;
            dst[1] = (src[1] as u32 * a / 255) as u8;
            dst[2] = (src[2] as u32 * a / 255) as u8;
            dst[3] = src[3];
        }
        p
    }

    pub fn from_tiny_skia(p: &tiny_skia::Pixmap) -> Self {
        let mut out = Pixmap {
            width: p.width(),
            height: p.height(),
            data: vec![0; (p.width() * p.height() * 4) as usize],
        };
        for (dst, src) in out.data.chunks_exact_mut(4).zip(p.data().chunks_exact(4)) {
            let a = src[3] as u32;
            if a == 0 {
                dst[..4].fill(0);
                continue;
            }
            dst[0] = (src[0] as u32 * 255 / a) as u8;
            dst[1] = (src[1] as u32 * 255 / a) as u8;
            dst[2] = (src[2] as u32 * 255 / a) as u8;
            dst[3] = src[3];
        }
        out
    }
}

fn blend_over(dst: &mut [u8], src: &[u8]) {
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

/// A loaded SVG document with KSvg-compatible element access.
pub struct SvgDoc {
    tree: usvg::Tree,
    /// Natural size of the SVG (in user units, as parsed).
    natural_size: (f32, f32),
    /// Resized size (KSvg's `Svg::resize`). Defaults to natural size.
    size: (f32, f32),
}

impl SvgDoc {
    /// Load an SVG from raw bytes (which may be gzipped `.svgz`).
    ///
    /// `stylesheet` is the generated `.ColorScheme-*` CSS that replaces the
    /// `current-color-scheme` style element, exactly like KSvg does.
    pub fn from_bytes(data: &[u8], stylesheet: &str) -> Result<Self, String> {
        let xml = if data.starts_with(&[0x1f, 0x8b]) {
            use std::io::Read;
            let mut decoder = flate2::read::GzDecoder::new(data);
            let mut out = Vec::new();
            decoder
                .read_to_end(&mut out)
                .map_err(|e| format!("failed to gunzip svgz: {e}"))?;
            out
        } else {
            data.to_vec()
        };

        let processed = if stylesheet.is_empty() {
            xml
        } else {
            substitute_color_scheme(&xml, stylesheet)?
        };

        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        let opt = usvg::Options {
            fontdb: Arc::new(db),
            ..usvg::Options::default()
        };
        let tree = usvg::Tree::from_data(&processed, &opt)
            .map_err(|e| format!("failed to parse svg: {e}"))?;
        let size = tree.size();
        let natural = (size.width(), size.height());
        Ok(SvgDoc {
            tree,
            natural_size: natural,
            size: natural,
        })
    }

    /// KSvg `Svg::resize`: set the target logical size.
    pub fn resize(&mut self, w: f32, h: f32) {
        self.size = (w, h);
    }

    /// KSvg `Svg::size()`: returns the (rounded) current size.
    pub fn size(&self) -> (f32, f32) {
        (self.size.0.round(), self.size.1.round())
    }

    /// Whether an element with the given id exists and has a valid bounding rect.
    pub fn has_element(&self, id: &str) -> bool {
        self.element_rect(id).is_some()
    }

    /// The element's bounding rect (including stroke) in canvas coordinates,
    /// scaled by `size / natural_size`, replicating KSvg's `elementRect`.
    ///
    /// Returns `None` when the element does not exist or has no area.
    pub fn element_rect(&self, id: &str) -> Option<tiny_skia::Rect> {
        let node = self.tree.node_by_id(id)?;
        let r = node.abs_stroke_bounding_box();
        let (cw, ch) = self.size();
        let (nw, nh) = self.natural_size;
        let dx = cw / nw;
        let dy = ch / nh;
        let scaled = tiny_skia::Rect::from_ltrb(
            r.left() * dx,
            r.top() * dy,
            r.right() * dx,
            r.bottom() * dy,
        )?;
        if scaled.width() <= 0.0 || scaled.height() <= 0.0 {
            return None;
        }
        Some(scaled)
    }

    /// KSvg `elementSize`: rounded element rect size.
    pub fn element_size(&self, id: &str) -> Option<(f32, f32)> {
        let r = self.element_rect(id)?;
        Some((r.width().round(), r.height().round()))
    }

    fn node(&self, id: &str) -> Option<&usvg::Node> {
        self.tree.node_by_id(id)
    }

    /// Render element `id` into a pixmap of exactly `target` device pixels.
    ///
    /// Replicates `QSvgRenderer::render(painter, elementId, bounds)`: the element's
    /// natural bounds are scaled (possibly non-uniformly) to fill the target rect.
    ///
    /// usvg wraps shapes carrying an element-level `opacity` in an implicit group
    /// that keeps the opacity; `node_by_id` returns the inner shape, so we render
    /// the enclosing group instead to preserve it.
    pub fn render_element(&self, id: &str, target: (u32, u32)) -> Option<Pixmap> {
        let node = self.node(id)?;
        let mut pixmap = tiny_skia::Pixmap::new(target.0, target.1)?;
        let ts = element_to_rect_transform(node, target);
        resvg::render_node(node, ts, &mut pixmap.as_mut())?;
        if let Some(wrapper) = find_opacity_wrapper(&self.tree, node) {
            let opacity = wrapper.opacity().get();
            if opacity < 1.0 {
                for px in pixmap.data_mut().chunks_exact_mut(4) {
                    px[3] = (px[3] as f32 * opacity) as u8;
                    let a = px[3] as u32;
                    px[0] = (px[0] as u32 * a / 255) as u8;
                    px[1] = (px[1] as u32 * a / 255) as u8;
                    px[2] = (px[2] as u32 * a / 255) as u8;
                }
            }
        }
        Some(Pixmap::from_tiny_skia(&pixmap))
    }

    /// Render the whole document at the given size.
    pub fn render_full(&self, target: (u32, u32)) -> Option<Pixmap> {
        let mut pixmap = tiny_skia::Pixmap::new(target.0, target.1)?;
        resvg::render(&self.tree, tiny_skia::Transform::identity(), &mut pixmap.as_mut());
        Some(Pixmap::from_tiny_skia(&pixmap))
    }
}

/// Build the transform that maps `node`'s canvas bounding box to the target rect.
///
/// `resvg::render_node` internally applies `pre_translate(-bbox)`, and the
/// renderer expects local geometry plus a transform that includes the node's
/// absolute transform. For group nodes the renderer pre-applies the group's own
/// local transform (`render_group`), so the passed transform must hold only the
/// ancestors' transforms (`abs_transform ∘ local⁻¹`); path nodes receive the
/// full `abs_transform`. The trailing `translate(+bbox)` cancels
/// `render_node`'s own `-bbox` shift.
fn element_to_rect_transform(node: &usvg::Node, target: (u32, u32)) -> tiny_skia::Transform {
    use tiny_skia::Transform;
    let bbox = node.abs_layer_bounding_box().unwrap();
    let stroke = node.abs_stroke_bounding_box();
    let abs_ts = node.abs_transform();
    let base = if let usvg::Node::Group(group) = node {
        group
            .transform()
            .invert()
            .map(|inv| abs_ts.pre_concat(inv))
            .unwrap_or(abs_ts)
    } else {
        abs_ts
    };
    let sx = target.0 as f32 / stroke.width();
    let sy = target.1 as f32 / stroke.height();
    Transform::from_scale(sx, sy)
        .pre_translate(-stroke.x(), -stroke.y())
        .pre_concat(base)
        .pre_translate(bbox.x(), bbox.y())
}

/// Replace the content of `<style id="current-color-scheme">` with `stylesheet`,
/// replicating KSvg's XML surgery in `SharedSvgRenderer::load`.
///
/// The original XML text is preserved verbatim; only the text between the
/// style element's opening and closing tags is swapped in (byte-range splice).
fn substitute_color_scheme(xml: &[u8], stylesheet: &str) -> Result<Vec<u8>, String> {
    let text = std::str::from_utf8(xml).map_err(|e| format!("svg is not utf-8: {e}"))?;
    let doc =
        roxmltree::Document::parse(text).map_err(|e| format!("failed to parse svg xml: {e}"))?;

    let style = doc
        .descendants()
        .find(|n| {
            n.is_element()
                && n.tag_name().name() == "style"
                && n.attribute("id") == Some("current-color-scheme")
        })
        .ok_or_else(|| "svg has no current-color-scheme style element".to_string())?;

    let elem_range = style.range();
    let (elem_start, elem_end) = (elem_range.start, elem_range.end);
    let open_tag_end = text[elem_start..elem_end]
        .find('>')
        .map(|i| elem_start + i + 1)
        .ok_or_else(|| "malformed style element".to_string())?;

    let mut out = String::with_capacity(text.len() + stylesheet.len());
    out.push_str(&text[..open_tag_end]);
    out.push_str(stylesheet);
    out.push_str("</style>");
    out.push_str(&text[elem_end..]);
    Ok(out.into_bytes())
}

/// Load an SVG file from disk.
pub fn load_svg_file(path: &Path, stylesheet: &str) -> Result<SvgDoc, String> {
    let data = std::fs::read(path).map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    SvgDoc::from_bytes(&data, stylesheet)
}

/// Encode a pixmap as PNG.
pub fn encode_png(pixmap: &Pixmap) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, pixmap.width, pixmap.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| format!("png write header: {e}"))?;
        writer
            .write_image_data(&pixmap.data)
            .map_err(|e| format!("png write data: {e}"))?;
    }
    Ok(out)
}


/// Find the implicit opacity wrapper group for `node` (usvg creates one when a
/// shape carries an element-level `opacity`). Returns `None` when the node is
/// not wrapped, in which case it should be rendered directly.
fn find_opacity_wrapper<'a>(tree: &'a usvg::Tree, node: &usvg::Node) -> Option<&'a usvg::Group> {
    if matches!(node, usvg::Node::Group(_)) {
        return None;
    }
    let id = node.id();
    if id.is_empty() {
        return None;
    }
    let mut result: Option<&'a usvg::Group> = None;
    let mut stack: Vec<&'a usvg::Group> = vec![tree.root()];
    while let Some(group) = stack.pop() {
        for child in group.children() {
            if child.id() == id && !matches!(child, usvg::Node::Group(_)) {
                result = Some(group);
                break;
            }
            if let usvg::Node::Group(g) = child {
                stack.push(g);
            }
        }
        if result.is_some() {
            break;
        }
    }
    result
}
