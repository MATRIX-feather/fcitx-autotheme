# fcitx-breeze-theme-generator

Generate Fcitx 5 Classic UI theme based on Plasma theme.

This is a pure-Rust reimplementation of the
[`fcitx5-plasma-theme-generator`](https://github.com/fcitx/fcitx5-configtool/blob/master/src/plasmathemegenerator/main.cpp)
utility from `fcitx5-configtool`. It renders a Plasma (KDE) desktop theme into
a Fcitx 5 Classic UI theme (a `theme.conf` plus PNG assets) without requiring
Qt, KDE Frameworks, Plasma, or fcitx at runtime.

The crate can be used both as a **library** (imported by other projects) and as
a **standalone executable** (`fcitx5-plasma-theme-generator`), which has no
runtime dependencies on external shared libraries.

## License

GPL-2.0-or-later, matching the original tool. See [LICENSES](./LICENSES).

## AI

This project uses AI all the way (Including most parts of this README), therefore I can't guarantee that this project won't have some silly problems.

## Usage

### Command line

```sh
# Generate a theme from the "default" (Breeze Light) Plasma theme
fcitx5-plasma-theme-generator -t default -o ~/.local/share/fcitx5/themes/plasma

# Use the global Plasma theme
fcitx5-plasma-theme-generator

# Monitor mode (used by fcitx5's ClassicUI watchdog): regenerate on theme
# changes and notify via a pipe
fcitx5-plasma-theme-generator --fd <fd> -o <output>
```

Options match the original tool:

| Option | Description |
|---|---|
| `-t, --theme <name>` | Plasma theme name (falls back to `default`) |
| `-o, --output <path>` | Output directory (default: `~/.local/share/fcitx5/themes/plasma`) |
| `--fd <fd>` | Monitor mode: watch a pipe, notify after each regeneration |
| `-h, --help` | Show help |
| `-v, --version` | Show version |

### As a library

```rust
use fcitx_breeze_theme_generator::{generate_theme, Theme};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let theme = Theme::new("default")?;
    generate_theme(&theme, std::path::Path::new("out"))?;
    Ok(())
}
```

## What it does

For a given Plasma theme (found under `plasma/desktoptheme/<name>` on the XDG
data paths), the generator:

1. Resolves the theme's color scheme (`colors` file → `kdeglobals` → Breeze
   Light defaults) and injects it into the theme SVGs via the
   `current-color-scheme` stylesheet substitution (KSvg semantics).
2. Renders `dialogs/background` as a 9-patch frame (`panel.png`, 200×200),
   with the `shadow` prefix frame and the blur `mask.png` when blur-behind is
   enabled.
3. Renders `widgets/viewitem` (hover/selected highlight) as `highlight.png`.
4. Renders `widgets/arrows` (`prev.png`, `next.png`, `arrow.png`),
   `widgets/checkmarks` (`radio.png`) and `widgets/line` (`line.png`).
5. Writes `theme.conf` in fcitx's INI format with margins derived from the
   SVG hint elements (identical to fcitx's `writeAsIni` output).

The frame geometry (hint-based margins, 9-patch compositing, tiling, alpha
mask) replicates KSvg's `FrameSvg` exactly, and the color roles replicate
`Plasma::Theme` / `KColorScheme` mappings. Output has been verified against the
original C++ tool's output pixel-for-pixel (alpha shapes 99.6–100% identical).

## Building

```sh
cargo build --release
```

The resulting binary links only against the C standard library.

## Architecture

| Module | Purpose |
|---|---|
| `theme` | Plasma theme discovery, metadata, SVG path resolution |
| `colorscheme` | KColorScheme-equivalent color resolution and stylesheet generation |
| `svg` | KSvg-compatible `Svg` / `FrameSvg` (margins, 9-patch, masks) |
| `render` | SVG loading (`.svg`/`.svgz`), element rendering, PNG encoding |
| `generator` | `generate_theme()` — the port of the original `generateTheme()` |
| `ini` | fcitx-compatible `RawConfig` + INI serializer |
| `color` | fcitx `Color` (16-bit channels, `#rrggbb` serialization) |

## Credits