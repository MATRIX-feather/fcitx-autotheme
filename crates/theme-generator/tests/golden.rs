//! End-to-end golden tests against the system's `default` Plasma theme with
//! a fixed synthetic color scheme, so results are fully deterministic.

use std::path::Path;

use image::GenericImageView;
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
HighlightBackgroundColor=#ff8800
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

fn generate() -> tempfile::TempDir {
    let colors = tempfile::NamedTempFile::new().expect("colors fixture");
    std::fs::write(colors.path(), FIXTURE_COLORS).expect("write fixture");
    let out = tempfile::tempdir().expect("out dir");
    ThemeGenerator::new(out.path())
        .with_theme_name("default")
        .with_colors_file(colors.path())
        .generate()
        .expect("generate");
    out
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
        conf.contains("HighlightBackgroundColor=#123456"),
        "accent must drive HighlightBackgroundColor:\n{conf}"
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

    // Breeze's viewitem hover frame paints with `ColorScheme-ButtonFocus`,
    // which resolves through the fallback chain to Window DecorationFocus;
    // the accent must recolor the rendered frame.
    let highlight = read_png(&out.path().join("highlight.png"));
    let center = highlight.get_pixel(100, 100).0;
    assert!(center[3] > 0, "hover center must be visible");
    assert!(
        center[0].abs_diff(0xe9) <= 2
            && center[1].abs_diff(0x3d) <= 2
            && center[2].abs_diff(0x58) <= 2,
        "highlight center must be the accent color, got {center:?}"
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
