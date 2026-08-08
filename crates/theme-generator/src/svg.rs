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

    /// The full style-replaced document bytes, at its native size.
    #[must_use]
    pub fn document(&self) -> &[u8] {
        &self.xml
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

    /// Emit a filtered, normalized SVG document.
    ///
    /// Only elements whose id matches `keep` survive (plus `defs`, the
    /// `current-color-scheme` style block and ancestor groups). The root
    /// `<svg>` gets explicit `width`/`height` set to `size` and a `viewBox`
    /// of `view_box`, so content maps onto the output at a known size. The
    /// style block content is already color-injected in `self.xml` and is
    /// preserved verbatim.
    pub fn emit(
        &self,
        keep: &dyn Fn(&str) -> bool,
        view_box: [f32; 4],
        size: (u32, u32),
    ) -> Result<Vec<u8>> {
        let text = std::str::from_utf8(&self.xml)
            .map_err(|e| Error::other(format!("invalid UTF-8 in SVG: {e}")))?;
        let doc = Document::parse(text)
            .map_err(|e| Error::other(format!("failed to parse SVG: {e}")))?;
        let root = doc.root_element();
        let mut writer = XmlWriter::new(Options {
            use_single_quote: false,
            ..Options::default()
        });
        writer.start_element("svg");
        write_xmlns(&mut writer, root);
        for attr in root.attributes() {
            if matches!(attr.name(), "width" | "height" | "viewBox") {
                continue;
            }
            write_attr(&mut writer, &attr);
        }
        writer.write_attribute("width", &size.0.to_string());
        writer.write_attribute("height", &size.1.to_string());
        let (x, y, w, h) = (view_box[0], view_box[1], view_box[2], view_box[3]);
        writer.write_attribute("viewBox", &format!("{x} {y} {w} {h}"));
        for child in root.children() {
            write_filtered_with(&mut writer, child, keep, "");
        }
        writer.end_element();
        Ok(writer.end_document().into_bytes())
    }

    /// Emit a single element (plus its defs and the color style block) as a
    /// standalone SVG rendered at `width` × `height`, with the element's
    /// bounding box as the viewBox. Action images (arrows, radio) render at
    /// this intrinsic size in fcitx5, so the size is the on-screen size.
    pub fn emit_element(&self, id: &str, width: u32, height: u32) -> Result<Vec<u8>> {
        let bbox = self
            .element_bbox(id)
            .ok_or_else(|| Error::missing_element(self.path.clone(), id))?;
        let keep = |candidate: &str| candidate == id;
        self.emit(&keep, bbox, (width, height))
    }

    /// Compose one or more 9-slice frames into a canonical-size SVG document,
    /// mirroring the raster composition in `frame.rs`: each slice's content
    /// is wrapped in a group whose transform maps the slice's absolute
    /// bounding box onto its region of the target canvas (corners at native
    /// size, edges scaled on one axis, center on both). Frames are painted
    /// in list order (later frames on top), each placed at its offset.
    ///
    /// `defs` and the injected color style block are preserved once. fcitx5
    /// later slices this document at the hint-derived `Margin`, reproducing
    /// the reference raster layout at any panel size.
    pub fn emit_composed(
        &self,
        frames: &[ComposedFrame<'_>],
        canvas: (u32, u32),
    ) -> Result<Vec<u8>> {
        let (cw, ch) = (canvas.0 as f32, canvas.1 as f32);
        let text = std::str::from_utf8(&self.xml)
            .map_err(|e| Error::other(format!("invalid UTF-8 in SVG: {e}")))?;
        let doc = Document::parse(text)
            .map_err(|e| Error::other(format!("failed to parse SVG: {e}")))?;
        let root = doc.root_element();
        let mut writer = XmlWriter::new(Options {
            use_single_quote: false,
            ..Options::default()
        });
        writer.start_element("svg");
        write_xmlns(&mut writer, root);
        for attr in root.attributes() {
            if matches!(attr.name(), "width" | "height" | "viewBox") {
                continue;
            }
            write_attr(&mut writer, &attr);
        }
        writer.write_attribute("width", &canvas.0.to_string());
        writer.write_attribute("height", &canvas.1.to_string());
        writer.write_attribute("viewBox", &format!("0 0 {cw} {ch}"));

        // defs are emitted wholesale (they already carry the color style
        // block when the theme puts it there); a style block outside any
        // defs is emitted separately, never duplicated.
        let defs: Vec<Node<'_, '_>> = root
            .descendants()
            .filter(|node| node.tag_name().name() == "defs")
            .collect();
        for node in &defs {
            write_node(&mut writer, *node, "");
        }
        for node in root.descendants() {
            if node.tag_name().name() == "style"
                && node.attribute("id") == Some("current-color-scheme")
                && !node.ancestors().any(|ancestor| ancestor.tag_name().name() == "defs")
            {
                write_node(&mut writer, node, "");
            }
        }

        for &(prefix, (fw, fh), (ox, oy)) in frames {
            let (sl, st, sr, sb) = self.frame_grid(prefix)?;
            let content_w = (fw as f32 - sl - sr).max(1.0);
            let content_h = (fh as f32 - st - sb).max(1.0);
            writer.start_element("g");
            if ox != 0.0 || oy != 0.0 {
                writer.write_attribute("transform", &format!("translate({ox} {oy})"));
            }
            let slice = |writer: &mut XmlWriter,
                         name: &str,
                         dx: f32,
                         dy: f32,
                         dw: f32,
                         dh: f32| -> Result<()> {
                let id = if prefix.is_empty() {
                    name.to_owned()
                } else {
                    format!("{prefix}-{name}")
                };
                let bbox = self
                    .element_bbox(&id)
                    .ok_or_else(|| Error::missing_element(self.path.clone(), &id))?;
                let node = find_node_by_id(&root, &id)
                    .ok_or_else(|| Error::missing_element(self.path.clone(), &id))?;
                let sx = dw / bbox[2].max(1.0);
                let sy = dh / bbox[3].max(1.0);
                writer.start_element("g");
                writer.write_attribute(
                    "transform",
                    &format!(
                        "translate({} {}) scale({sx} {sy})",
                        dx - bbox[0] * sx,
                        dy - bbox[1] * sy
                    ),
                );
                write_node_path(writer, root, node);
                writer.end_element();
                Ok(())
            };
            slice(&mut writer, "top", sl, 0.0, content_w, st)?;
            slice(&mut writer, "bottom", sl, fh as f32 - sb, content_w, sb)?;
            slice(&mut writer, "left", 0.0, st, sl, content_h)?;
            slice(&mut writer, "right", fw as f32 - sr, st, sr, content_h)?;
            slice(&mut writer, "center", sl, st, content_w, content_h)?;
            slice(&mut writer, "topleft", 0.0, 0.0, sl, st)?;
            slice(&mut writer, "topright", fw as f32 - sr, 0.0, sr, st)?;
            slice(&mut writer, "bottomleft", 0.0, fh as f32 - sb, sl, sb)?;
            slice(&mut writer, "bottomright", fw as f32 - sr, fh as f32 - sb, sr, sb)?;
            writer.end_element();
        }
        writer.end_element();
        Ok(writer.end_document().into_bytes())
    }

    /// The frame border thicknesses (left, top, right, bottom) from the
    /// slice elements' sizes, in document units.
    fn frame_grid(&self, prefix: &str) -> Result<(f32, f32, f32, f32)> {
        let full = |name: &str| -> String {
            if prefix.is_empty() {
                name.to_owned()
            } else {
                format!("{prefix}-{name}")
            }
        };
        let width = |name: &str| -> Result<f32> {
            let bbox = self
                .element_bbox(&full(name))
                .ok_or_else(|| Error::missing_element(self.path.clone(), &full(name)))?;
            Ok(bbox[2])
        };
        let height = |name: &str| -> Result<f32> {
            let bbox = self
                .element_bbox(&full(name))
                .ok_or_else(|| Error::missing_element(self.path.clone(), &full(name)))?;
            Ok(bbox[3])
        };
        Ok((width("left")?, height("top")?, width("right")?, height("bottom")?))
    }
}

/// A frame to compose: element prefix, target size, and offset within the
/// canvas.
pub type ComposedFrame<'a> = (&'a str, (u32, u32), (f32, f32));

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

/// Write the namespace declarations for an emitted SVG: the default SVG
/// namespace (required for a valid, renderable document) plus `xmlns:xlink`
/// when the document uses `xlink:href` references. Editor namespaces
/// (inkscape, sodipodi, ...) are intentionally dropped together with their
/// attributes.
fn write_xmlns(writer: &mut XmlWriter, root: Node<'_, '_>) {
    writer.write_attribute("xmlns", "http://www.w3.org/2000/svg");
    if root.namespaces().any(|ns| ns.name() == Some("xlink")) {
        writer.write_attribute("xmlns:xlink", "http://www.w3.org/1999/xlink");
    }
}

/// Write an attribute with a properly qualified name. Attributes from
/// foreign namespaces (editor junk like `inkscape:*` / `sodipodi:*`) are
/// dropped; `xlink:` and `xml:` prefixes are preserved. Returns whether the
/// attribute was written.
fn write_attr(writer: &mut XmlWriter, attr: &roxmltree::Attribute<'_, '_>) -> bool {
    let name = match attr.namespace() {
        None => attr.name().to_owned(),
        Some("http://www.w3.org/1999/xlink") => format!("xlink:{}", attr.name()),
        Some("http://www.w3.org/XML/1998/namespace") => format!("xml:{}", attr.name()),
        Some(_) => return false,
    };
    writer.write_attribute(&name, attr.value());
    true
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
    write_xmlns(&mut writer, root);
    for attr in root.attributes() {
        write_attr(&mut writer, &attr);
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
    write_xmlns(&mut writer, root);
    for attr in root.attributes() {
        write_attr(&mut writer, &attr);
    }
    for child in root.children() {
        write_filtered(&mut writer, child, prefix, "");
    }
    writer.end_element();
    Ok(writer.end_document().into_bytes())
}

/// Decide whether a node survives frame filtering.
fn keep_node_with(node: Node<'_, '_>, keep: &dyn Fn(&str) -> bool) -> bool {
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
        && keep(id)
    {
        return true;
    }
    // Keep ancestors of kept elements.
    node.children().any(|child| keep_node_with(child, keep))
}

/// Serialize a node if it survives filtering, recursing with the same
/// predicate so wrapper groups do not drag non-kept children along. `defs`
/// content (gradients, masks, clips) is written wholesale: frame filtering
/// must never strip the defs a slice element references.
fn write_filtered(writer: &mut XmlWriter, node: Node<'_, '_>, prefix: &str, stylesheet: &str) {
    write_filtered_with(writer, node, &|id| matches_prefix(id, prefix), stylesheet);
}

/// Predicate-based variant of [`write_filtered`], used for emitting SVGs
/// whose keep-set spans multiple prefixes (e.g. a panel frame plus its
/// shadow) or single elements (icons).
fn write_filtered_with(
    writer: &mut XmlWriter,
    node: Node<'_, '_>,
    keep: &dyn Fn(&str) -> bool,
    stylesheet: &str,
) {
    if !keep_node_with(node, keep) {
        return;
    }
    if node.tag_name().name() == "defs" {
        write_node(writer, node, stylesheet);
        return;
    }
    // Matched elements are written wholesale (including their geometry
    // children), mirroring KSvg `boundsOnElement`, which renders the slice's
    // full content.
    if let Some(id) = node.attribute("id")
        && keep(id)
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
        write_style_block(writer, node, stylesheet);
        return;
    }
    writer.start_element(tag);
    for attr in node.attributes() {
        write_attr(writer, &attr);
    }
    for child in node.children() {
        write_filtered_with(writer, child, keep, stylesheet);
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
pub(crate) fn matches_prefix(id: &str, prefix: &str) -> bool {
    if prefix.is_empty() {
        FRAME_SLICES.contains(&id)
    } else {
        FRAME_SLICES
            .iter()
            .any(|slice| id == &format!("{prefix}-{slice}"))
    }
}

/// Serialize the `current-color-scheme` style element, replacing its content
/// with `stylesheet` (or keeping the original text when empty).
fn write_style_block(writer: &mut XmlWriter, node: Node<'_, '_>, stylesheet: &str) {
    writer.start_element("style");
    for attr in node.attributes() {
        write_attr(writer, &attr);
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
        write_style_block(writer, node, stylesheet);
        return;
    }
    writer.start_element(tag);
    for attr in node.attributes() {
        write_attr(writer, &attr);
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

/// Find a node by id anywhere in the document.
fn find_node_by_id<'a, 'input>(root: &'a Node<'a, 'input>, id: &str) -> Option<Node<'a, 'input>> {
    root.descendants().find(|node| node.attribute("id") == Some(id))
}

/// Serialize `target` together with its ancestor chain, dropping sibling
/// subtrees. Ancestor ids are dropped so the same wrapper group is not
/// emitted with duplicate ids when several slices share a parent.
fn write_node_path(writer: &mut XmlWriter, node: Node<'_, '_>, target: Node<'_, '_>) {
    if node == target {
        write_node(writer, node, "");
        return;
    }
    if node.is_comment() || node.is_text() {
        return;
    }
    if node.tag_name().name() == "defs" || node.tag_name().name() == "style" {
        return;
    }
    if !node.descendants().any(|n| n == target) {
        return;
    }
    writer.start_element(node.tag_name().name());
    for attr in node.attributes() {
        if attr.name() == "id" {
            continue;
        }
        write_attr(writer, &attr);
    }
    for child in node.children() {
        write_node_path(writer, child, target);
    }
    writer.end_element();
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

    #[test]
    fn emit_composed_places_slices_with_transforms() {
        let raw = br#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="30">
  <style id="current-color-scheme">.ColorScheme-Background{color:#ff0000;}</style>
  <defs><linearGradient id="g"/></defs>
  <g id="layer" transform="translate(5, 5)">
    <g id="top"><rect width="30" height="4"/></g>
    <g id="bottom"><rect width="30" height="4"/></g>
    <g id="left"><rect width="4" height="22"/></g>
    <g id="right"><rect width="4" height="22"/></g>
    <g id="center"><rect width="22" height="14"/></g>
    <g id="topleft"><rect width="4" height="4"/></g>
    <g id="topright"><rect width="4" height="4"/></g>
    <g id="bottomleft"><rect width="4" height="4"/></g>
    <g id="bottomright"><rect width="4" height="4"/></g>
  </g>
</svg>"#;
        let replaced =
            replace_style_block(raw, ".ColorScheme-Background{color:#112233;}", "t").expect("replace");
        let tree = usvg::Tree::from_data(&replaced, &usvg::Options::default()).expect("tree");
        let svg = PlasmaSvg {
            path: PathBuf::from("t.svg"),
            xml: replaced,
            full: tree,
            frame_trees: HashMap::new(),
        };
        let emitted = svg
            .emit_composed(&[("", (20, 20), (2.0, 3.0))], (20, 20))
            .expect("emit");
        let text = String::from_utf8_lossy(&emitted);
        assert!(text.contains("xmlns=\"http://www.w3.org/2000/svg\""));
        assert!(text.contains("width=\"20\""));
        assert!(text.contains("viewBox=\"0 0 20 20\""));
        assert!(text.contains("translate(2 3)"));
        assert!(text.contains("id=\"g\""), "defs must survive");
        assert!(text.contains(".ColorScheme-Background{color:#112233;}"), "colors lost");
        assert!(!text.contains("id=\"layer\""), "ancestor id must be dropped");
        assert!(!text.contains("inkscape:"), "editor junk must be dropped");
        let tree = usvg::Tree::from_data(&emitted, &usvg::Options::default()).expect("parses");
        // The composed frame fills the canvas: center rect now spans the
        // region between the borders.
        assert_eq!(tree.size().width(), 20.0);
    }

    #[test]
    fn emit_element_scales_bbox_to_target_size() {
        let raw = br#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24">
  <style id="current-color-scheme">.ColorScheme-Text{color:#000000;}</style>
  <defs><linearGradient id="g"/></defs>
  <path id="left-arrow" d="M0 0h16v16H0z" class="ColorScheme-Text"/>
</svg>"#;
        let replaced =
            replace_style_block(raw, ".ColorScheme-Text{color:#aabbcc;}", "t").expect("replace");
        let tree = usvg::Tree::from_data(&replaced, &usvg::Options::default()).expect("tree");
        let svg = PlasmaSvg {
            path: PathBuf::from("t.svg"),
            xml: replaced,
            full: tree,
            frame_trees: HashMap::new(),
        };
        let emitted = svg.emit_element("left-arrow", 22, 22).expect("emit");
        let text = String::from_utf8_lossy(&emitted);
        assert!(text.contains("xmlns=\"http://www.w3.org/2000/svg\""));
        assert!(text.contains("width=\"22\""));
        assert!(text.contains("viewBox=\"0 0 16 16\""));
        assert!(text.contains("id=\"left-arrow\""));
        assert!(text.contains("id=\"g\""), "defs must survive");
        assert!(text.contains(".ColorScheme-Text{color:#aabbcc;}"), "colors lost");
    }
}

/// Debug helper: render a tree at native size (used by tests).
#[doc(hidden)]
pub fn render_for_debug(tree: &usvg::Tree) -> image::RgbaImage {
    render_scaled(tree, 1.0).expect("render")
}
