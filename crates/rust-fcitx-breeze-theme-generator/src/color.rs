// SPDX-FileCopyrightText: 2026 Fcitx Breeze Theme Generator Authors
//
// SPDX-License-Identifier: GPL-2.0-or-later
//
//! `fcitx::Color` equivalent, replicating the exact `toString()` output so the
//! generated `theme.conf` matches the original tool byte for byte.
//!
//! See `fcitx5/src/lib/fcitx-utils/color.cpp`.

/// A color with 16-bit-per-channel precision, like `fcitx::Color`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    red: u16,
    green: u16,
    blue: u16,
    alpha: u16,
}

impl Color {
    /// Create a color from 0.0–1.0 floats, replicating `fcitx::Color::setRedF`
    /// (float multiply by 65535, truncated to `u16`).
    pub fn from_rgba_f32(r: f32, g: f32, b: f32, a: f32) -> Self {
        Color {
            red: (r * 65535.0) as u16,
            green: (g * 65535.0) as u16,
            blue: (b * 65535.0) as u16,
            alpha: (a * 65535.0) as u16,
        }
    }

    /// Create a color from 8-bit channels (as read from a Plasma `colors` file).
    pub fn from_rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        // QColor::redF() == red()/255.0 computed in double precision, then
        // implicitly narrowed to float when passed to fcitx::Color::setRedF.
        Color::from_rgba_f32(
            (r as f64 / 255.0) as f32,
            (g as f64 / 255.0) as f32,
            (b as f64 / 255.0) as f32,
            (a as f64 / 255.0) as f32,
        )
    }

    /// Serialize as `#rrggbb`, or `#rrggbbaa` when alpha is not 255.
    ///
    /// Replicates `fcitx::Color::toString()`: the 16-bit channels are shifted
    /// right by 8 and rendered as hex; a trailing `ff` alpha byte is dropped.
    pub fn to_hex_string(&self) -> String {
        let mut result = String::with_capacity(9);
        result.push('#');
        for &channel in &[self.red, self.green, self.blue, self.alpha] {
            let v = channel >> 8;
            result.push(hex_nibble(v >> 4));
            result.push(hex_nibble(v & 0xF));
        }
        if result.ends_with("ff") {
            result.truncate(result.len() - 2);
        }
        result
    }
}

fn hex_nibble(v: u16) -> char {
    match v {
        0..=9 => (b'0' + v as u8) as char,
        _ => (b'a' + (v - 10) as u8) as char,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_hex() {
        assert_eq!(Color::from_rgba8(0xFC, 0xFC, 0xFC, 0xFF).to_hex_string(), "#fcfcfc");
        assert_eq!(Color::from_rgba8(0xFF, 0xFF, 0xFF, 0xFF).to_hex_string(), "#ffffff");
        assert_eq!(Color::from_rgba8(0x31, 0x68, 0x38, 0xFF).to_hex_string(), "#316838");
        assert_eq!(
            Color::from_rgba8(0x12, 0x34, 0x56, 0x78).to_hex_string(),
            "#12345678"
        );
    }
}
