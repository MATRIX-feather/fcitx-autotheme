//! Regression: theme hint debug colors must never leak into rendered PNGs.
//!
//! Runs against whatever Plasma theme is active on the system and skips when
//! none is usable (e.g. non-KDE environments), so the test passes anywhere.

use image::GenericImageView;
use theme_generator::ThemeGenerator;

#[test]
fn no_hint_colors_in_active_theme_panel() {
    let out = tempfile::tempdir().expect("out dir");
    let generator = ThemeGenerator::new(out.path());
    if generator.generate().is_err() {
        // No usable Plasma theme on this system; nothing to assert.
        return;
    }
    let panel = image::open(out.path().join("panel.png")).expect("panel").to_rgba8();
    // Hint margins are #ff00ff, hint insets #00ff00; both must be absent.
    let hints = panel
        .pixels()
        .filter(|p| p.0[3] > 0 && (p.0[0], p.0[1], p.0[2]) == (0xff, 0x00, 0xff) || (p.0[0], p.0[1], p.0[2]) == (0x00, 0xff, 0x00))
        .count();
    assert_eq!(hints, 0, "panel contains {hints} hint debug pixels");
    // Sanity: the panel actually rendered content.
    let opaque = panel.pixels().filter(|p| p.0[3] > 0).count();
    assert!(opaque > 20_000, "panel mostly empty: {opaque}");
}
