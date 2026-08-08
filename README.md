# fcitx-autotheme

Watch Plasma theme and color-scheme changes, then automatically regenerate the
fcitx5 theme to match — no manual intervention required.

This project is made because at the time being, fcitx5 in flatpak doesn't automatically match KDE Plasma's current theme.

## AI

This project uses AI all the way: ChatGPT for the core part of the script, and DeepSeek V4 Pro + [OpenCode](https://github.com/anomalyco/opencode) + [oh-my-openagent](https://github.com/code-yeongyu/oh-my-openagent) for the Rust part and README. README contains manual changes.

One thing to note is that I have no Rust programming experience (in fact, I've learned almost nothing :D), so I can't guarantee that this project won't have some silly problems.

## What it does

Whenever you switch your Plasma color scheme or global theme, `fcitx-autotheme`
detects the change via D-Bus and:

1. Generates a matching fcitx5 classicui theme in-process
2. Reloads the fcitx5 `classicui` addon so the new theme takes effect immediately

This replicates the behavior of the [original bash script](watch-theme-then-update-fcitx-theme),
but in Rust with proper async I/O, graceful shutdown, and debounced signal handling.
The theme generation itself is a Rust port of `fcitx5-plasma-theme-generator`
(the fcitx5-configtool KCM utility): it resolves the Plasma theme's color
scheme, renders the theme's SVG frames to PNGs, and writes `theme.conf`.

## Workspace layout

```
crates/
├── autotheme/                          # the D-Bus watcher daemon (binary: fcitx-autotheme)
└── rust-fcitx-breeze-theme-generator/  # in-process Plasma → fcitx5 theme generator (library + CLI)
```

## Build this project

```bash
cargo build --release
```

### Prerequisites

- A running D-Bus session bus
- KDE Plasma desktop (for the D-Bus signals it monitors)

## Usage

```
fcitx-autotheme [OPTIONS]

Options:
  -w, --wait-time <MILLIS>  Debounce wait time in milliseconds [default: 100]
  -h, --help                Print help
```

With a custom debounce (e.g., wait 500 ms before reacting):

```bash
fcitx-autotheme --wait-time 500
```

## How it works

`fcitx-autotheme` listens for two D-Bus signals:

| Signal | Trigger |
|---|---|
| `org.freedesktop.portal.Settings.SettingChanged` with namespace `org.kde.kdeglobals.General` and key `ColorScheme` | KDE color scheme change |
| `org.kde.kconfig.notify.ConfigChanged` on path `/plasmarc` | Plasma theme/config change |

Signals are **debounced**: when a change is detected, the daemon waits for a
configurable quiet period (default 100 ms). Any additional signals arriving
during this window reset the timer. Once the window elapses without new signals,
the theme is regenerated exactly once. This prevents redundant processing when
multiple settings change at the same time (e.g., switching both color scheme
and Plasma theme).

On shutdown (SIGINT / SIGTERM), the daemon exits gracefully.

## How the generator works

`fcitx-breeze-theme-generator` (crates/rust-fcitx-breeze-theme-generator)
reproduces the algorithm of `fcitx5-plasma-theme-generator` (upstream:
fcitx/fcitx5-configtool, `src/plasmathemegenerator/main.cpp`,
GPL-2.0-or-later):

1. Load the Plasma theme (`default`) and resolve its color scheme — the
   theme's `colors` file, falling back to the active KDE scheme from
   `kdeglobals`, then to the Breeze Light defaults.
2. Load the theme's SVGs (`dialogs/background`, `widgets/viewitem`,
   `widgets/arrows`, ...) and inject the color scheme into the
   `current-color-scheme` style block. The 9-slice frames are rendered to
   bitmaps (`panel.png`, `mask.png`, `highlight.png`, icons) with usvg/resvg.
3. Write `theme.conf` in the fcitx5 classicui format.

## License

See LICENSES under each crates.
