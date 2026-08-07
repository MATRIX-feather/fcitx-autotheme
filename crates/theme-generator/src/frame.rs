//! 9-slice frame composition, ported from `KSvg` `FrameSvg`.
//!
//! The generator's panel/mask/highlight images are Plasma theme frames:
//! a 9-slice grid (corners painted at their native size, edges tiled, center
//! stretched or tiled) composed onto a target canvas. This module computes
//! the grid from element bounding boxes and the margins from `hint-*`
//! elements, exactly like `FrameSvgPrivate::updateSizes` / `sectionRect`.

use image::imageops::FilterType;
use image::RgbaImage;

use crate::svg::{render_scaled, PlasmaSvg};
use crate::{Error, Result};

/// Element sizes defining the 9-slice grid (document coordinates).
#[derive(Debug, Clone, Copy)]
pub struct Grid {
    /// Width of the left border element.
    pub left: f32,
    /// Width of the right border element.
    pub right: f32,
    /// Height of the top border element.
    pub top: f32,
    /// Height of the bottom border element.
    pub bottom: f32,
}

/// Bounding boxes of the nine frame elements (document coordinates).
#[derive(Debug, Clone, Copy)]
struct Elements {
    top: [f32; 4],
    bottom: [f32; 4],
    left: [f32; 4],
    right: [f32; 4],
    center: [f32; 4],
    topleft: [f32; 4],
    topright: [f32; 4],
    bottomleft: [f32; 4],
    bottomright: [f32; 4],
}

/// A composited frame, mirroring `FrameSvg` for one element prefix.
pub struct Frame<'a> {
    svg: &'a mut PlasmaSvg,
    prefix: String,
    target_w: u32,
    target_h: u32,
    grid: Grid,
    margins: (f32, f32, f32, f32),
    tile_center: bool,
    elements: Elements,
}

impl<'a> Frame<'a> {
    /// Compute the frame grid and margins for `prefix`.
    pub fn new(svg: &'a mut PlasmaSvg, prefix: &str, target_w: u32, target_h: u32) -> Result<Self> {
        let full = |name: &str| -> String {
            if prefix.is_empty() {
                name.to_owned()
            } else {
                format!("{prefix}-{name}")
            }
        };
        let bbox = |name: &str| -> Result<[f32; 4]> {
            svg.element_bbox(&full(name))
                .ok_or_else(|| Error::missing_element(svg.path().to_path_buf(), &full(name)))
        };
        let elements = Elements {
            top: bbox("top")?,
            bottom: bbox("bottom")?,
            left: bbox("left")?,
            right: bbox("right")?,
            center: bbox("center")?,
            topleft: bbox("topleft")?,
            topright: bbox("topright")?,
            bottomleft: bbox("bottomleft")?,
            bottomright: bbox("bottomright")?,
        };
        let grid = Grid {
            left: elements.left[2],
            right: elements.right[2],
            top: elements.top[3],
            bottom: elements.bottom[3],
        };
        // Margins: hint elements win over element sizes (`updateSizes`).
        let hint = |name: &str| svg.element_bbox(&full(name));
        let top_margin = hint("hint-top-margin")
            .map_or(grid.top, |b| b[3]);
        let bottom_margin = hint("hint-bottom-margin")
            .map_or(grid.bottom, |b| b[3]);
        let left_margin = hint("hint-left-margin")
            .map_or(grid.left, |b| b[2]);
        let right_margin = hint("hint-right-margin")
            .map_or(grid.right, |b| b[2]);
        let margins = (left_margin, top_margin, right_margin, bottom_margin);
        // `hint-tile-center` (prefixed or not) selects center tiling.
        let tile_center = svg.has_element(&full("hint-tile-center"))
            || (prefix.is_empty() && svg.has_element("hint-tile-center"));
        Ok(Self {
            svg,
            prefix: prefix.to_owned(),
            target_w,
            target_h,
            grid,
            margins,
            tile_center,
            elements,
        })
    }

    /// The content margins (left, top, right, bottom).
    #[must_use]
    pub const fn margins(&self) -> (f32, f32, f32, f32) {
        self.margins
    }

    /// Compose the frame onto the target canvas.
    pub fn render(self) -> Result<RgbaImage> {
        let tree = self.svg.frame_tree(&self.prefix)?;
        let native = render_scaled(tree, 1.0)?;
        Ok(compose(&native, &self))
    }
}

/// Compose the 9-slice frame from the native render.
fn compose(native: &RgbaImage, frame: &Frame<'_>) -> RgbaImage {
    let g = frame.grid;
    let left = g.left.round().max(0.0) as u32;
    let right = g.right.round().max(0.0) as u32;
    let top = g.top.round().max(0.0) as u32;
    let bottom = g.bottom.round().max(0.0) as u32;
    let content_w = frame.target_w.saturating_sub(left + right);
    let content_h = frame.target_h.saturating_sub(top + bottom);

    let mut canvas = RgbaImage::new(frame.target_w, frame.target_h);

    // Center: stretched (default) or tiled (`hint-tile-center`).
    let center = crop_bbox(native, &frame.elements.center);
    if !center.is_empty() && content_w > 0 && content_h > 0 {
        if frame.tile_center {
            tile(&mut canvas, &center, left, top, content_w, content_h);
        } else {
            let scaled = image::imageops::resize(&center, content_w, content_h, FilterType::Triangle);
            crate::paste(&mut canvas, &scaled, left, top);
        }
    }

    // Corners at native size.
    let tl = crop_bbox(native, &frame.elements.topleft);
    crate::paste(&mut canvas, &tl, 0, 0);
    let tr = crop_bbox(native, &frame.elements.topright);
    crate::paste(&mut canvas, &tr, frame.target_w - tr.width(), 0);
    let bl = crop_bbox(native, &frame.elements.bottomleft);
    crate::paste(&mut canvas, &bl, 0, frame.target_h - bl.height());
    let br = crop_bbox(native, &frame.elements.bottomright);
    crate::paste(
        &mut canvas,
        &br,
        frame.target_w - br.width(),
        frame.target_h - br.height(),
    );

    // Borders tiled over their sections.
    let top_side = crop_bbox(native, &frame.elements.top);
    tile(&mut canvas, &top_side, left, 0, content_w, top);
    let bottom_side = crop_bbox(native, &frame.elements.bottom);
    tile(
        &mut canvas,
        &bottom_side,
        left,
        frame.target_h - bottom,
        content_w,
        bottom,
    );
    let left_side = crop_bbox(native, &frame.elements.left);
    tile(&mut canvas, &left_side, 0, top, left, content_h);
    let right_side = crop_bbox(native, &frame.elements.right);
    tile(
        &mut canvas,
        &right_side,
        frame.target_w - right,
        top,
        right,
        content_h,
    );

    canvas
}

/// Crop a bounding box from a raster (clamped to the image).
fn crop_bbox(image: &RgbaImage, bbox: &[f32; 4]) -> RgbaImage {
    let left = bbox[0].round().max(0.0) as u32;
    let top = bbox[1].round().max(0.0) as u32;
    let width = bbox[2].round().max(1.0) as u32;
    let height = bbox[3].round().max(1.0) as u32;
    let left = left.min(image.width());
    let top = top.min(image.height());
    let width = width.min(image.width() - left);
    let height = height.min(image.height() - top);
    image::imageops::crop_imm(image, left, top, width, height).to_image()
}

/// Tile `src` over the `(x, y, w, h)` rect on `dst`, including partial tiles.
fn tile(dst: &mut RgbaImage, src: &RgbaImage, x: u32, y: u32, w: u32, h: u32) {
    if src.is_empty() || w == 0 || h == 0 {
        return;
    }
    let (sw, sh) = (src.width(), src.height());
    let mut ty = y;
    while ty < y + h {
        let tile_h = (y + h - ty).min(sh);
        let mut tx = x;
        while tx < x + w {
            let tile_w = (x + w - tx).min(sw);
            if tile_w == sw && tile_h == sh {
                crate::paste(dst, src, tx, ty);
            } else {
                let region = image::imageops::crop_imm(src, 0, 0, tile_w, tile_h).to_image();
                crate::paste(dst, &region, tx, ty);
            }
            tx += tile_w;
        }
        ty += tile_h;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_covers_rect_with_partials() {
        let src = RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255]));
        let mut dst = RgbaImage::new(5, 3);
        tile(&mut dst, &src, 0, 0, 5, 3);
        // Every pixel of the 5x3 area must be filled.
        for y in 0..3 {
            for x in 0..5 {
                assert_eq!(dst.get_pixel(x, y).0, [255, 0, 0, 255], "at {x},{y}");
            }
        }
    }

    #[test]
    fn crop_clamps_to_image() {
        let img = RgbaImage::new(10, 10);
        let c = crop_bbox(&img, &[8.0, 8.0, 10.0, 10.0]);
        assert_eq!(c.width(), 2);
        assert_eq!(c.height(), 2);
    }
}
