//! Plasma theme SVG loading, color substitution, and rendering.
//!
//! Reimplements the parts of `KSvg` that the generator needs:
//!
//! - Locating theme SVG assets (`svg/<name>.svgz` or `<name>.svgz`).
//! - Injecting the theme colors into the `current-color-scheme` style block.
//! - Two usvg trees per asset: the full tree (style replaced, used for
//!   element bounding boxes, margins and hint detection) and per-frame
//!   filtered trees (only the 9-slice elements of a given prefix, used for
//!   rendering — `KSvg` never renders the whole document).
//! - Element bounding boxes and raster rendering via usvg/resvg.

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;
use image::RgbaImage;
use roxmltree::{Document, Node};
use xmlwriter::{Options, XmlWriter};

use crate::theme_resolver::Theme;
use crate::{Error, Result};

/// A loaded and preprocessed Plasma theme SVG.
pub struct PlasmaSvg {
    path: PathBuf,
    /// Style-replaced, unfiltered XML (source for per-frame filtering).
    xml: Vec<u8>,
    /// Full tree for measurements (bboxes, hints).
    full: usvg::Tree,
    /// Per-prefix filtered trees for rendering, built lazily.
    frame_trees: HashMap<String, usvg::Tree>,
}

impl PlasmaSvg {
    /// Load a theme SVG asset, e.g. `dialogs/background`, and preprocess it
    /// with the given stylesheet (colors injected into the style block).
    pub fn load(theme: &Theme, name: &str, stylesheet: &str) -> Result<Self> {
        let path = resolve_asset(&theme.dir, name).ok_or_else(|| {
            Error::other(format!(
                "SVG asset not found: {name} in {}",
                theme.dir.display()
            ))
        })?;
        let raw = read_asset(&path)?;
        let xml = replace_style_block(&raw, stylesheet, name)?;
        let full = parse_tree(&xml).map_err(|source| Error::svg(path.clone(), source))?;
        Ok(Self {
            path,
            xml,
            full,
            frame_trees: HashMap::new(),
        })
    }

    /// The asset file path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether any element id equals `prefix` or starts with `prefix-`.
    pub fn has_element_prefix(&self, prefix: &str) -> bool {
        element_exists_prefix(self.full.root(), prefix)
    }

    /// Whether an element with the given id exists in the full tree.
    pub fn has_element(&self, id: &str) -> bool {
        find_node(self.full.root(), id).is_some()
    }

    /// Absolute bounding box of an element in document coordinates.
    pub fn element_bbox(&self, id: &str) -> Option<[f32; 4]> {
        let node = find_node(self.full.root(), id)?;
        let rect = node.abs_bounding_box();
        Some([rect.x(), rect.y(), rect.width(), rect.height()])
    }

    /// Build a [`crate::frame::Frame`] for the given element prefix.
    pub fn frame(&mut self, prefix: &str, width: u32, height: u32) -> Result<crate::frame::Frame<'_>> {
        crate::frame::Frame::new(self, prefix, width, height)
    }

    /// The filtered render tree for a frame prefix.
    pub(crate) fn frame_tree(&mut self, prefix: &str) -> Result<&usvg::Tree> {
        let key = prefix.to_owned();
        if !self.frame_trees.contains_key(&key) {
            let filtered = filter_frame(&self.xml, prefix)?;
            let tree =
                parse_tree(&filtered).map_err(|source| Error::svg(self.path.clone(), source))?;
            self.frame_trees.insert(key.clone(), tree);
        }
        self.frame_trees
            .get(&key)
            .ok_or_else(|| Error::other("frame tree missing after insert"))
    }

    /// Render the full document at its native size.
    pub fn render_native(&self) -> Result<RgbaImage> {
        render_scaled(&self.full, 1.0)
    }

    /// Render an element scaled (uniformly) to fit `target` and centered.
    pub fn render_element(&self, id: &str, target_w: u32, target_h: u32) -> Result<RgbaImage> {
        let bbox = self
            .element_bbox(id)
            .ok_or_else(|| Error::missing_element(self.path.clone(), id))?;
        let scale = (target_w as f32 / bbox[2].max(1.0)).min(target_h as f32 / bbox[3].max(1.0));
        render_element_at_scale(self, &bbox, scale, target_w, target_h)
    }

    /// Render an element into a canvas of the SVG's current size (the
    /// natural size, since frames are never resized here).
    pub fn render_element_native(&self, id: &str) -> Result<RgbaImage> {
        let bbox = self
            .element_bbox(id)
            .ok_or_else(|| Error::missing_element(self.path.clone(), id))?;
        let size = self.full.size();
        let w = size.width().round().max(1.0) as u32;
        let h = size.height().round().max(1.0) as u32;
        render_element_at_scale(self, &bbox, 1.0, w, h)
    }
}

/// Render an element: scale the whole document uniformly, then crop the
/// element's region, then center it on a `target_w`×`target_h` canvas.
fn render_element_at_scale(
    svg: &PlasmaSvg,
    bbox: &[f32; 4],
    scale: f32,
    target_w: u32,
    target_h: u32,
) -> Result<RgbaImage> {
    let scaled = render_scaled(&svg.full, scale)?;
    let x0 = (bbox[0] * scale).round().max(0.0) as u32;
    let y0 = (bbox[1] * scale).round().max(0.0) as u32;
    let w = (bbox[2] * scale).round().max(1.0) as u32;
    let h = (bbox[3] * scale).round().max(1.0) as u32;
    let x0 = x0.min(scaled.width());
    let y0 = y0.min(scaled.height());
    let w = w.min(scaled.width() - x0);
    let h = h.min(scaled.height() - y0);
    let cropped = image::imageops::crop_imm(&scaled, x0, y0, w, h).to_image();
    let mut canvas = RgbaImage::new(target_w, target_h);
    let ox = target_w.saturating_sub(cropped.width()) / 2;
    let oy = target_h.saturating_sub(cropped.height()) / 2;
    crate::paste(&mut canvas, &cropped, ox, oy);
    Ok(canvas)
}

/// Render the tree at `scale` into an RGBA image (unpremultiplied).
pub(crate) fn render_scaled(tree: &usvg::Tree, scale: f32) -> Result<RgbaImage> {
    let size = tree.size();
    let width = (size.width() * scale).ceil().max(1.0) as u32;
    let height = (size.height() * scale).ceil().max(1.0) as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
        .ok_or(Error::Alloc { width, height })?;
    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(tree, transform, &mut pixmap.as_mut());
    Ok(unpremultiply(&pixmap))
}

/// Convert a `tiny_skia` (premultiplied) pixmap into a straight-alpha RGBA image.
fn unpremultiply(pixmap: &resvg::tiny_skia::Pixmap) -> RgbaImage {
    let (width, height) = (pixmap.width(), pixmap.height());
    let data = pixmap.data();
    let mut out = RgbaImage::new(width, height);
    for (pixel, chunk) in out.pixels_mut().zip(data.chunks_exact(4)) {
        let [red, green, blue, alpha] = chunk.try_into().unwrap_or([0, 0, 0, 0]);
        let (red, green, blue) = if alpha == 0 {
            (0, 0, 0)
        } else {
            let alpha16 = u32::from(alpha);
            (
                (u32::from(red) * 255 / alpha16) as u8,
                (u32::from(green) * 255 / alpha16) as u8,
                (u32::from(blue) * 255 / alpha16) as u8,
            )
        };
        *pixel = image::Rgba([red, green, blue, alpha]);
    }
    out
}

/// Resolve an asset file: `<theme>/svg/<name>.<ext>` first (Plasma 6 layout),
/// then `<theme>/<name>.<ext>`, for `.svgz`, `.svg` and `.png`.
fn resolve_asset(theme_dir: &Path, name: &str) -> Option<PathBuf> {
    let candidates = [theme_dir.join("svg").join(name), theme_dir.join(name)];
    for base in candidates {
        for ext in ["svgz", "svg", "png"] {
            let path = base.with_extension(ext);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

/// Read an asset, decompressing `.svgz`.
fn read_asset(path: &Path) -> Result<Vec<u8>> {
    let raw = fs::read(path).map_err(|source| Error::io(path.to_path_buf(), source))?;
    if path.extension().is_some_and(|e| e == "svgz") {
        let mut decoder = GzDecoder::new(&raw[..]);
        let mut out = Vec::new();
        decoder
            .read_to_end(&mut out)
            .map_err(|source| Error::io(path.to_path_buf(), source))?;
        Ok(out)
    } else {
        Ok(raw)
    }
}

/// Replace the `current-color-scheme` style block content with `stylesheet`.
fn replace_style_block(raw: &[u8], stylesheet: &str, name: &str) -> Result<Vec<u8>> {
    let doc = Document::parse(std::str::from_utf8(raw).map_err(|e| Error::other(format!("invalid UTF-8 in {name}: {e}")))?)
        .map_err(|e| Error::other(format!("failed to parse {name}: {e}")))?;
    let root = doc.root_element();
    let mut writer = XmlWriter::new(Options {
        use_single_quote: false,
        ..Options::default()
    });
    writer.start_element("svg");
    let mut seen = Vec::new();
    for attr in root.attributes() {
        if seen.contains(&attr.name()) {
            continue;
        }
        seen.push(attr.name());
        writer.write_attribute(attr.name(), attr.value());
    }
    for child in root.children() {
        write_node(&mut writer, child, stylesheet);
    }
    writer.end_element();
    Ok(writer.end_document().into_bytes())
}

/// Filter a style-replaced SVG down to one frame's elements.
///
/// The keep set mirrors `KSvg` `FrameSvg`: a frame renders exactly its nine
/// slice elements (see [`matches_prefix`]). `defs` and the color style block
/// are always kept (defs are required by gradients and masks). Wrapper groups
/// (e.g. Inkscape `layer1`) that contain kept elements are kept as well, but
/// their own non-kept children (hints, debug helpers) are still dropped.
fn filter_frame(xml: &[u8], prefix: &str) -> Result<Vec<u8>> {
    let doc = Document::parse(std::str::from_utf8(xml).map_err(|e| Error::other(format!("invalid UTF-8 in SVG: {e}")))?)
        .map_err(|e| Error::other(format!("failed to filter SVG: {e}")))?;
    let root = doc.root_element();
    let mut writer = XmlWriter::new(Options {
        use_single_quote: false,
        ..Options::default()
    });
    writer.start_element("svg");
    let mut seen = Vec::new();
    for attr in root.attributes() {
        if seen.contains(&attr.name()) {
            continue;
        }
        seen.push(attr.name());
        writer.write_attribute(attr.name(), attr.value());
    }
    for child in root.children() {
        write_filtered(&mut writer, child, prefix, "");
    }
    writer.end_element();
    Ok(writer.end_document().into_bytes())
}

/// Decide whether a node survives frame filtering.
fn keep_node(node: Node<'_, '_>, prefix: &str) -> bool {
    if node.is_comment() || node.is_text() {
        return false;
    }
    if node.tag_name().name() == "defs" {
        return true;
    }
    if node.tag_name().name() == "style" && node.attribute("id") == Some("current-color-scheme") {
        return true;
    }
    if let Some(id) = node.attribute("id")
        && matches_prefix(id, prefix)
    {
        return true;
    }
    // Keep ancestors of kept elements.
    node.children().any(|child| keep_node(child, prefix))
}

/// Serialize a node if it survives filtering, recursing with the same
/// predicate so wrapper groups do not drag non-kept children along. `defs`
/// content (gradients, masks, clips) is written wholesale: frame filtering
/// must never strip the defs a slice element references.
fn write_filtered(writer: &mut XmlWriter, node: Node<'_, '_>, prefix: &str, stylesheet: &str) {
    if !keep_node(node, prefix) {
        return;
    }
    if node.tag_name().name() == "defs" {
        write_node(writer, node, stylesheet);
        return;
    }
    // Slice elements are written wholesale (including their geometry
    // children), mirroring KSvg `boundsOnElement`, which renders the slice's
    // full content.
    if let Some(id) = node.attribute("id")
        && matches_prefix(id, prefix)
    {
        write_node(writer, node, stylesheet);
        return;
    }
    if node.is_comment() {
        return;
    }
    if node.is_text() {
        writer.write_text(node.text().unwrap_or_default());
        return;
    }
    let tag = node.tag_name().name();
    if tag == "style" && node.attribute("id") == Some("current-color-scheme") {
        writer.start_element("style");
        let mut seen = Vec::new();
        for attr in node.attributes() {
            if seen.contains(&attr.name()) {
                continue;
            }
            seen.push(attr.name());
            writer.write_attribute(attr.name(), attr.value());
        }
        if stylesheet.is_empty() {
            for child in node.children() {
                if child.is_text() {
                    writer.write_text(child.text().unwrap_or_default());
                }
            }
        } else {
            writer.write_text(stylesheet);
        }
        writer.end_element();
        return;
    }
    writer.start_element(tag);
    let mut seen = Vec::new();
    for attr in node.attributes() {
        if seen.contains(&attr.name()) {
            continue;
        }
        seen.push(attr.name());
        writer.write_attribute(attr.name(), attr.value());
    }
    for child in node.children() {
        write_filtered(writer, child, prefix, stylesheet);
    }
    writer.end_element();
}

/// The nine 9-slice frame element suffixes, mirroring `KSvg` `FrameSvg`.
const FRAME_SLICES: &[&str] = &[
    "top", "bottom", "left", "right", "center", "topleft", "topright", "bottomleft",
    "bottomright",
];

/// `KSvg` prefix matching: a frame renders exactly its nine slice elements.
/// For an empty prefix the base ids are used; for a non-empty prefix the
/// `prefix-` variants. Everything else (hints, overlays, debug helpers) is
/// never part of a frame render and must be filtered out.
fn matches_prefix(id: &str, prefix: &str) -> bool {
    if prefix.is_empty() {
        FRAME_SLICES.contains(&id)
    } else {
        FRAME_SLICES
            .iter()
            .any(|slice| id == &format!("{prefix}-{slice}"))
    }
}

/// Serialize a node (and descendants) into the writer, replacing the style
/// block content with `stylesheet`.
fn write_node(writer: &mut XmlWriter, node: Node<'_, '_>, stylesheet: &str) {
    if node.is_comment() {
        return;
    }
    if node.is_text() {
        writer.write_text(node.text().unwrap_or_default());
        return;
    }
    let tag = node.tag_name().name();
    if tag == "style" && node.attribute("id") == Some("current-color-scheme") {
        writer.start_element("style");
        let mut seen = Vec::new();
        for attr in node.attributes() {
            if seen.contains(&attr.name()) {
                continue;
            }
            seen.push(attr.name());
            writer.write_attribute(attr.name(), attr.value());
        }
        if stylesheet.is_empty() {
            for child in node.children() {
                if child.is_text() {
                    writer.write_text(child.text().unwrap_or_default());
                }
            }
        } else {
            writer.write_text(stylesheet);
        }
        writer.end_element();
        return;
    }
    writer.start_element(tag);
    let mut seen = Vec::new();
    for attr in node.attributes() {
        if seen.contains(&attr.name()) {
            continue;
        }
        seen.push(attr.name());
        writer.write_attribute(attr.name(), attr.value());
    }
    for child in node.children() {
        write_node(writer, child, stylesheet);
    }
    writer.end_element();
}

/// Parse SVG bytes into a usvg tree.
fn parse_tree(xml: &[u8]) -> std::result::Result<usvg::Tree, usvg::Error> {
    usvg::Tree::from_data(xml, &usvg::Options::default())
}

/// Depth-first search for a node by id.
pub(crate) fn find_node<'a>(group: &'a usvg::Group, id: &str) -> Option<&'a usvg::Node> {
    for child in group.children() {
        if child.id() == id {
            return Some(child);
        }
        if let usvg::Node::Group(g) = child
            && let Some(found) = find_node(g, id)
        {
            return Some(found);
        }
    }
    None
}

/// Whether any node id equals `prefix` or starts with `prefix-`.
fn element_exists_prefix(group: &usvg::Group, prefix: &str) -> bool {
    let dash = format!("{prefix}-");
    group.children().iter().any(|child| {
        let id = child.id();
        if id == prefix || id.starts_with(&dash) {
            return true;
        }
        if let usvg::Node::Group(g) = child {
            return element_exists_prefix(g, prefix);
        }
        false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_block_replacement() {
        let raw = br#"<?xml version="1.0"?>
<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
  <style id="current-color-scheme">.ColorScheme-Background{color:#ff0000;}</style>
  <defs><linearGradient id="g"><stop offset="0"/></linearGradient></defs>
  <g id="top"><rect class="ColorScheme-Background" width="10" height="2"/></g>
</svg>"#;
        let replaced =
            replace_style_block(raw, ".ColorScheme-Background{color:#112233;}", "t").expect("replace");
        let text = String::from_utf8_lossy(&replaced);
        assert!(text.contains(".ColorScheme-Background{color:#112233;}"));
        assert!(text.contains("id=\"top\""));
        assert!(!text.contains("#ff0000"));
        let tree = usvg::Tree::from_data(&replaced, &usvg::Options::default()).expect("parse");
        assert!(find_node(&tree.root(), "top").is_some());
    }

    #[test]
    fn frame_filtering_keeps_prefix_and_defs() {
        let raw = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
  <defs><linearGradient id="g"/></defs>
  <g id="shadow-left"><rect width="1" height="10"/></g>
  <g id="shadow-center"><rect width="8" height="10"/></g>
  <rect id="hint-tile-center" width="1" height="1"/>
</svg>"#;
        let filtered = filter_frame(raw, "shadow").expect("filter");
        let text = String::from_utf8_lossy(&filtered);
        assert!(text.contains("id=\"shadow-left\""));
        assert!(text.contains("id=\"shadow-center\""));
        assert!(!text.contains("hint-tile-center"));
        assert!(text.contains("id=\"g\""));
    }

    #[test]
    fn empty_prefix_filter_excludes_unprefixed_hints() {
        let raw = br#"<svg xmlns="http://www.w3.org/2000/svg" width="120" height="64">
  <g id="top"><rect width="32" height="6"/></g>
  <rect id="hint-top-margin" x="20" y="10" width="4" height="4" style="fill:#ff00ff"/>
  <rect id="hint-left-margin" x="0" y="30" width="4" height="4" style="fill:#ff00ff"/>
  <g id="shadow-top"><rect width="32" height="16"/></g>
</svg>"#;
        let filtered = filter_frame(raw, "").expect("filter");
        let text = String::from_utf8_lossy(&filtered);
        assert!(text.contains("id=\"top\""));
        assert!(!text.contains("hint-top-margin"), "top margin hint leaked");
        assert!(!text.contains("hint-left-margin"), "left margin hint leaked");
        assert!(!text.contains("shadow-top"), "shadow slice leaked into bg frame");
        eprintln!("filtered: {text}");
    }

    #[test]
    fn filtered_bg_tree_has_no_hint_pixels() {
        let theme = crate::theme_resolver::resolve_theme(Some("default")).expect("theme");
        let css = ".ColorScheme-Text{color:#aabbcc;}";
        let mut svg = PlasmaSvg::load(&theme, "dialogs/background", css).expect("load");
        let debug_xml = filter_frame(&svg.xml, "").expect("filter");
        std::fs::write("/tmp/opencode/filtered-bg.svg", &debug_xml).expect("write");
        let tree = svg.frame_tree("").expect("frame tree");
        let native = render_scaled(tree, 1.0).expect("render");
        let probe = native.get_pixel(20, 10).0;
        let center = native.get_pixel(24, 33).0;
        eprintln!("pixel at (20,10) = {probe:?}, center-ish = {center:?}");
        // Dump which elements survived the filter.
        fn dump(group: &usvg::Group, out: &mut Vec<String>) {
            for child in group.children() {
                let id = child.id();
                if !id.is_empty() {
                    out.push(id.to_owned());
                }
                if let usvg::Node::Group(g) = child {
                    dump(g, out);
                }
            }
        }
        let mut ids = Vec::new();
        dump(tree.root(), &mut ids);
        eprintln!("filtered bg ids: {ids:?}");
        // hint-top-margin sits at doc (20,10,4,4); the filtered bg tree must
        // not render it (no bright magenta).
        for y in 8..12 {
            for x in 18..26 {
                let p = native.get_pixel(x, y).0;
                assert!(
                    !((p[0], p[1], p[2]) == (0xff, 0x00, 0xff) && p[3] > 0),
                    "hint leaked at ({x},{y})"
                );
            }
        }
    }

    #[test]
    fn frame_filtering_excludes_hints_from_prefix_frames() {
        let raw = br#"<svg xmlns="http://www.w3.org/2000/svg" width="120" height="64">
  <g id="shadow-top"><rect width="32" height="16"/></g>
  <rect id="shadow-hint-top-margin" x="76" y="0" width="2" height="10" style="fill:#ff00ff"/>
  <rect id="shadow-hint-top-inset" x="76" y="0" width="2" height="1" style="fill:#00ff00"/>
  <rect id="hint-top-margin" x="20" y="10" width="4" height="4" style="fill:#ff00ff"/>
</svg>"#;
        let filtered = filter_frame(raw, "shadow").expect("filter");
        let text = String::from_utf8_lossy(&filtered);
        assert!(text.contains("id=\"shadow-top\""));
        assert!(!text.contains("shadow-hint-top-margin"), "margin hint leaked");
        assert!(!text.contains("shadow-hint-top-inset"), "inset hint leaked");
        assert!(!text.contains("hint-top-margin"), "unprefixed hint leaked");
    }
}

/// Debug helper: render a tree at native size (used by tests).
#[doc(hidden)]
pub fn render_for_debug(tree: &usvg::Tree) -> image::RgbaImage {
    render_scaled(tree, 1.0).expect("render")
}
