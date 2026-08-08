//! End-to-end golden tests against the system's `default` Plasma theme with
//! a fixed synthetic color scheme, so results are fully deterministic.

use std::path::Path;

use image::GenericImageView;
use theme_generator::OutputFormat;
use theme_generator::ThemeGenerator;

const FIXTURE_COLORS: &str = r#"[Colors:Window]
Background=#112233
Foreground=#aabbcc
DecorationFocus=#556677
DecorationHover=#445566
[Colors:View]
Background=#334455
Foreground=#ddeeff
DecorationFocus=#667788
DecorationHover=#556677
Highlight=#ff8800
HighlightedText=#ffffff
[Colors:Button]
Background=#223344
Foreground=#bbccee
DecorationFocus=#667788
DecorationHover=#556677
[Colors:Selection]
Background=#ff8800
Foreground=#ffffff
[Colors:Tooltip]
Background=#223344
Foreground=#bbccee
[Colors:Complementary]
Background=#223344
Foreground=#bbccee
[Colors:Header]
Background=#223344
Foreground=#bbccee
"#;

/// The expected theme.conf produced from the current system `default` Plasma
/// theme with the fixture color scheme. Margins come from the theme's hint
/// elements (ksvg `updateSizes`): ShadowMargin = shadow hint height (10),
/// and ContentMargin/Background/Margin add the shadow offset to the bg hint
/// height (4 + 10 = 14), matching main.cpp lines 256-259.
const EXPECTED_THEME_CONF: &str = "[Metadata]
Name=Plasma
Version=1
Author=Fcitx
Description=\"Theme generated from Plasma Theme default\"

[InputPanel]
NormalColor=#aabbcc
HighlightCandidateColor=#aabbcc
HighlightColor=#ffffff
HighlightBackgroundColor=#e57a00
PageButtonAlignment=\"Last Candidate\"
BlurMask=mask.png
EnableBlur=True

[InputPanel/ContentMargin]
Left=14
Top=14
Right=14
Bottom=14

[InputPanel/ShadowMargin]
Left=10
Top=10
Right=10
Bottom=10

[InputPanel/Background]
Image=panel.png

[InputPanel/Background/Margin]
Left=14
Top=14
Right=14
Bottom=14

[InputPanel/Highlight]
Image=highlight.png

[InputPanel/Highlight/Margin]
Left=5
Top=5
Right=5
Bottom=5

[InputPanel/TextMargin]
Left=5
Top=7
Right=5
Bottom=7

[InputPanel/PrevPage]
Image=prev.png

[InputPanel/NextPage]
Image=next.png

[Menu]
Spacing=2.000000

[Menu/ContentMargin]
Left=14
Top=14
Right=14
Bottom=14

[Menu/Background]
Image=panel.png

[Menu/Background/Margin]
Left=14
Top=14
Right=14
Bottom=14

[Menu/Highlight]
Image=highlight.png

[Menu/Highlight/Margin]
Left=5
Top=5
Right=5
Bottom=5

[Menu/TextMargin]
Left=5
Top=5
Right=5
Bottom=5

[Menu/SubMenu]
Image=arrow.png

[Menu/CheckBox]
Image=radio.png

[Menu/Separator]
Image=line.png

";

fn generate_with(format: OutputFormat) -> tempfile::TempDir {
    let colors = tempfile::NamedTempFile::new().expect("colors fixture");
    std::fs::write(colors.path(), FIXTURE_COLORS).expect("write fixture");
    let out = tempfile::tempdir().expect("out dir");
    ThemeGenerator::new(out.path())
        .with_theme_name("default")
        .with_colors_file(colors.path())
        .with_format(format)
        .generate()
        .expect("generate");
    out
}

fn generate() -> tempfile::TempDir {
    generate_with(OutputFormat::Png)
}

fn read_png(path: &Path) -> image::RgbaImage {
    image::open(path).expect("open png").to_rgba8()
}

/// The theme SVG hint elements use bright debug colors; if any of them leak
/// into a rendered image the filtering is broken.
fn assert_no_hint_colors(image: &image::RgbaImage, name: &str) {
    for pixel in image.pixels() {
        let [r, g, b, a] = pixel.0;
        if a == 0 {
            continue;
        }
        assert!(
            !((r, g, b) == (0xff, 0x00, 0xff) || (r, g, b) == (0x00, 0xff, 0x00)),
            "{name} contains a hint debug color at ({r},{g},{b})"
        );
    }
}

#[test]
fn golden_theme_conf_matches_reference_structure() {
    let out = generate();
    let actual = std::fs::read_to_string(out.path().join("theme.conf")).expect("read conf");
    assert_eq!(actual, EXPECTED_THEME_CONF);
}

#[test]
fn golden_panel_has_frame_colors_and_size() {
    let out = generate();
    let panel = read_png(&out.path().join("panel.png"));
    assert_eq!(panel.dimensions(), (200, 200));
    // The frame background is [Colors:Window] Background at 0.85 opacity.
    // usvg rasterizes premultiplied, so RGB may differ from Qt's straight
    // alpha by ±1 per channel; compare with tolerance.
    let center = panel.get_pixel(100, 100).0;
    let (r, g, b, a) = (center[0], center[1], center[2], center[3]);
    assert!(r.abs_diff(0x11) <= 2, "r={r}");
    assert!(g.abs_diff(0x22) <= 2, "g={g}");
    assert!(b.abs_diff(0x33) <= 2, "b={b}");
    assert!(a == 216 || a == 217, "alpha={a}");
    // Extreme corners are transparent (rounded shadow).
    assert_eq!(panel.get_pixel(2, 2).0, [0, 0, 0, 0]);
    // The shadow gradient is present near the top edge.
    assert_ne!(panel.get_pixel(100, 8).0[3], 0);
    // Hint debug colors (#ff00ff margins, #00ff00 insets) must never leak
    // into the rendered panel.
    assert_no_hint_colors(&panel, "panel");
}

#[test]
fn golden_mask_is_black_shape() {
    let out = generate();
    let mask = read_png(&out.path().join("mask.png"));
    assert_eq!(mask.dimensions(), (200, 200));
    let mut opaque = 0;
    for pixel in mask.pixels() {
        if pixel.0[3] > 0 {
            opaque += 1;
            assert_eq!(pixel.0, [0, 0, 0, 255], "mask pixels must be solid black");
        }
    }
    // The mask region roughly matches the panel's opaque region.
    assert!(opaque > 20_000, "mask too small: {opaque}");
    assert_no_hint_colors(&mask, "mask");
}

#[test]
fn golden_icons_have_reference_sizes_and_colors() {
    let out = generate();
    for (name, (w, h)) in [
        ("prev.png", (22, 22)),
        ("next.png", (22, 22)),
        ("arrow.png", (16, 16)),
        ("radio.png", (16, 16)),
    ] {
        let img = read_png(&out.path().join(name));
        assert_eq!(img.dimensions(), (w, h), "{name} size");
    }
    let line = read_png(&out.path().join("line.png"));
    assert_eq!(line.dimensions(), (3, 1), "line.png size");
    // Arrows are drawn with the `.ColorScheme-Text` class = Window Foreground.
    let prev = read_png(&out.path().join("prev.png"));
    let has_arrow_color = prev.pixels().any(|p| {
        let [r, g, b, a] = p.0;
        a > 0 && (r, g, b) == (0xaa, 0xbb, 0xcc)
    });
    assert!(has_arrow_color, "prev.png must use the window foreground color");
}

#[test]
fn golden_highlight_is_200x200_with_hover_content() {
    let out = generate();
    let highlight = read_png(&out.path().join("highlight.png"));
    assert_eq!(highlight.dimensions(), (200, 200));
    let opaque = highlight.pixels().filter(|p| p.0[3] > 0).count();
    assert!(opaque > 10_000, "highlight mostly empty: {opaque}");
}

#[test]
fn accent_color_overrides_highlight_background() {
    let colors = tempfile::NamedTempFile::new().expect("colors fixture");
    std::fs::write(colors.path(), FIXTURE_COLORS).expect("write fixture");
    let out = tempfile::tempdir().expect("out dir");
    ThemeGenerator::new(out.path())
        .with_theme_name("default")
        .with_colors_file(colors.path())
        .with_accent_color(theme_generator::colors::Color::opaque(0x12, 0x34, 0x56))
        .generate()
        .expect("generate");
    let conf = std::fs::read_to_string(out.path().join("theme.conf")).expect("read conf");
    assert!(
        conf.contains("HighlightBackgroundColor=#0d2e50"),
        "accent must drive HighlightBackgroundColor (deepened 10%):\n{conf}"
    );
    // Non-accent colors are untouched.
    assert!(
        conf.contains("NormalColor=#aabbcc"),
        "non-accent colors must be unchanged:\n{conf}"
    );
}

#[test]
fn accent_color_recolors_decoration_pngs() {
    let colors = tempfile::NamedTempFile::new().expect("colors fixture");
    std::fs::write(colors.path(), FIXTURE_COLORS).expect("write fixture");
    let accent = theme_generator::colors::Color::opaque(0xe9, 0x3d, 0x58);

    let out = tempfile::tempdir().expect("out dir");
    ThemeGenerator::new(out.path())
        .with_theme_name("default")
        .with_colors_file(colors.path())
        .with_accent_color(accent)
        .generate()
        .expect("generate");

    // Breeze's viewitem hover frame paints with `ColorScheme-ButtonFocus`
    // (the accent is injected into the Button set too); the rendered frame
    // must carry the accent, deepened 10% (#e93d58 -> #d92e4a).
    let highlight = read_png(&out.path().join("highlight.png"));
    let center = highlight.get_pixel(100, 100).0;
    assert!(center[3] > 0, "hover center must be visible");
    assert!(
        center[0].abs_diff(0xd9) <= 2
            && center[1].abs_diff(0x2e) <= 2
            && center[2].abs_diff(0x4a) <= 2,
        "highlight center must be the deepened accent color, got {center:?}"
    );

    // The radiobutton outer ring uses the same role; its pixels must shift
    // from the scheme's Window DecorationFocus (#556677, r<=87) to
    // accent-driven tones (red-dominant).
    let radio = read_png(&out.path().join("radio.png"));
    assert!(
        radio.pixels().any(|p| p.0[3] > 0 && p.0[0] >= 150),
        "radio must contain accent-driven (red-dominant) pixels"
    );
}

/// Render an emitted SVG at its intrinsic size (unpremultiplied RGBA).
fn render_svg(data: &[u8]) -> image::RgbaImage {
    let tree = usvg::Tree::from_data(data, &usvg::Options::default()).expect("parse svg");
    let w = tree.size().width().round().max(1.0) as u32;
    let h = tree.size().height().round().max(1.0) as u32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h).expect("pixmap");
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::default(),
        &mut pixmap.as_mut(),
    );
    let mut out = image::RgbaImage::new(w, h);
    for (pixel, chunk) in out.pixels_mut().zip(pixmap.data().chunks_exact(4)) {
        let [r, g, b, a] = chunk.try_into().unwrap_or([0, 0, 0, 0]);
        let (r, g, b) = if a == 0 {
            (0, 0, 0)
        } else {
            let a16 = u32::from(a);
            (
                (u32::from(r) * 255 / a16) as u8,
                (u32::from(g) * 255 / a16) as u8,
                (u32::from(b) * 255 / a16) as u8,
            )
        };
        *pixel = image::Rgba([r, g, b, a]);
    }
    out
}

#[test]
fn svg_theme_conf_differs_only_in_image_extensions() {
    let png =
        std::fs::read_to_string(generate().path().join("theme.conf")).expect("read png conf");
    let svg = std::fs::read_to_string(generate_with(OutputFormat::Svg).path().join("theme.conf"))
        .expect("read svg conf");
    assert_eq!(
        svg.replace(".svg", ".png"),
        png,
        "SVG theme.conf must differ from PNG only in image extensions"
    );
}

#[test]
fn svg_files_are_valid_documents_with_namespace() {
    // Regression: emitted SVGs must declare the SVG namespace and a valid
    // `version`, or strict renderers (browsers, VS Code, librsvg) treat them
    // as broken and render nothing.
    let out = generate_with(OutputFormat::Svg);
    let names = [
        "panel.svg",
        "mask.svg",
        "highlight.svg",
        "prev.svg",
        "next.svg",
        "arrow.svg",
        "radio.svg",
        "line.svg",
    ];
    for name in names {
        let text = std::fs::read_to_string(out.path().join(name)).expect(name);
        assert!(
            text.contains("xmlns=\"http://www.w3.org/2000/svg\""),
            "{name} missing the SVG namespace"
        );
        assert!(
            !text.contains("version=\"1.4.2"),
            "{name} leaks an Inkscape version string into the svg version attribute"
        );
        // Strict parsers must accept the document.
        usvg::Tree::from_data(text.as_bytes(), &usvg::Options::default())
            .unwrap_or_else(|e| panic!("{name} does not parse: {e}"));
    }
}

#[test]
fn svg_panel_renders_clean_and_matches_frame_colors() {
    let out = generate_with(OutputFormat::Svg);
    let data = std::fs::read(out.path().join("panel.svg")).expect("panel.svg");
    let panel = render_svg(&data);
    assert!(
        panel.width() >= 100 && panel.height() >= 100,
        "panel too small: {}x{}",
        panel.width(),
        panel.height()
    );
    // The frame center carries the Window Background color at ~0.85 alpha.
    let (w, h) = (panel.width(), panel.height());
    let center = panel.get_pixel(w / 2, h / 2).0;
    assert!(
        center[0].abs_diff(0x11) <= 3
            && center[1].abs_diff(0x22) <= 3
            && center[2].abs_diff(0x33) <= 3,
        "center {center:?}"
    );
    assert!(center[3] > 200, "center alpha {}", center[3]);
    // A corner is transparent (the shadow's rounded outer corner).
    assert_eq!(panel.get_pixel(2, 2).0, [0, 0, 0, 0]);
    // Hint debug colors must never leak into the emitted SVG.
    assert_no_hint_colors(&panel, "panel.svg");
}

#[test]
fn svg_mask_renders_opaque_shape_without_hints() {
    let out = generate_with(OutputFormat::Svg);
    let data = std::fs::read(out.path().join("mask.svg")).expect("mask.svg");
    let mask = render_svg(&data);
    let opaque = mask.pixels().filter(|p| p.0[3] > 0).count();
    assert!(
        opaque * 2 > (mask.width() * mask.height()) as usize,
        "mask too small: {opaque} of {}x{}",
        mask.width(),
        mask.height()
    );
    assert_no_hint_colors(&mask, "mask.svg");
}

#[test]
fn svg_highlight_renders_with_hover_content() {
    let out = generate_with(OutputFormat::Svg);
    let data = std::fs::read(out.path().join("highlight.svg")).expect("highlight.svg");
    let highlight = render_svg(&data);
    let opaque = highlight.pixels().filter(|p| p.0[3] > 0).count();
    assert!(
        opaque * 2 > (highlight.width() * highlight.height()) as usize,
        "highlight mostly empty: {opaque} of {}x{}",
        highlight.width(),
        highlight.height()
    );
    assert_no_hint_colors(&highlight, "highlight.svg");
}

#[test]
fn svg_icons_have_reference_sizes() {
    let out = generate_with(OutputFormat::Svg);
    for (name, w, h) in [
        ("prev.svg", 22, 22),
        ("next.svg", 22, 22),
        ("arrow.svg", 16, 16),
        ("radio.svg", 16, 16),
    ] {
        let tree = usvg::Tree::from_data(
            &std::fs::read(out.path().join(name)).expect(name),
            &usvg::Options::default(),
        )
        .expect("parse icon");
        assert_eq!(
            (
                tree.size().width().round() as u32,
                tree.size().height().round() as u32
            ),
            (w, h),
            "{name} size"
        );
    }
    // The separator keeps its natural size.
    let line_tree = usvg::Tree::from_data(
        &std::fs::read(out.path().join("line.svg")).expect("line.svg"),
        &usvg::Options::default(),
    )
    .expect("parse line");
    assert_eq!(line_tree.size().width().round() as u32, 3, "line width");
    // Arrows are drawn with the `.ColorScheme-Text` class = Window Foreground.
    let prev = render_svg(&std::fs::read(out.path().join("prev.svg")).expect("prev.svg"));
    let has_arrow_color = prev.pixels().any(|p| {
        let [r, g, b, a] = p.0;
        a > 0 && (r, g, b) == (0xaa, 0xbb, 0xcc)
    });
    assert!(has_arrow_color, "prev.svg must use the window foreground color");
}
