// SPDX-FileCopyrightText: 2022~2022 CSSlayer <wengxt@gmail.com>
// SPDX-FileCopyrightText: 2026 Fcitx Breeze Theme Generator Authors
//
// SPDX-License-Identifier: GPL-2.0-or-later
//
//! Generate Fcitx 5 Classic UI theme based on Plasma theme.
//!
//! This crate is a pure-Rust reimplementation of the
//! `fcitx5-plasma-theme-generator` utility from `fcitx5-configtool`.
//!
//! It can be used both as a library (see [`generator`]) and as a standalone
//! executable (`fcitx5-plasma-theme-generator`), which has no runtime
//! dependencies on external shared libraries.

pub mod color;
pub mod colorscheme;
pub mod generator;
pub mod ini;
pub mod render;
pub mod svg;
pub mod theme;

pub use generator::{generate_theme, GenerateResult};
pub use theme::Theme;
