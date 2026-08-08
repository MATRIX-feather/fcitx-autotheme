// SPDX-FileCopyrightText: 2026 Fcitx Breeze Theme Generator Authors
//
// SPDX-License-Identifier: GPL-2.0-or-later
//
//! A minimal, insertion-ordered configuration tree and an INI serializer that
//! produces output byte-identical to fcitx5's `RawConfig` + `writeAsIni`
//! (see `fcitx5/src/lib/fcitx-config/iniparser.cpp` and `rawconfig.cpp`).

use std::collections::HashMap;
use std::fmt::Write as _;

/// A node in the configuration tree.
///
/// Mirrors `fcitx::RawConfig`: each node has a name, an optional value and an
/// insertion-ordered list of sub items. Paths use `/` as the separator.
#[derive(Debug, Default, Clone)]
pub struct RawConfig {
    name: String,
    value: String,
    /// Sub items in insertion order.
    sub_items: Vec<RawConfig>,
    /// name -> index into `sub_items` for O(1) lookup.
    index: HashMap<String, usize>,
}

impl RawConfig {
    /// Create a new empty node.
    pub fn new(name: impl Into<String>) -> Self {
        RawConfig {
            name: name.into(),
            ..Default::default()
        }
    }

    /// The node's name (the key under its parent).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The node's value, if any.
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Set the node's value.
    pub fn set_value(&mut self, value: impl Into<String>) {
        self.value = value.into();
    }

    /// True if this node has any sub items.
    pub fn has_sub_items(&self) -> bool {
        !self.sub_items.is_empty()
    }

    /// Direct sub items in insertion order.
    pub fn sub_items(&self) -> &[RawConfig] {
        &self.sub_items
    }

    /// Get a direct child by name.
    pub fn child(&self, name: &str) -> Option<&RawConfig> {
        self.index.get(name).map(|&i| &self.sub_items[i])
    }

    /// Get a child by path (`"A/B/C"`), returning `None` if missing.
    pub fn get(&self, path: &str) -> Option<&RawConfig> {
        let mut cur = self;
        for part in path.split('/') {
            cur = cur.child(part)?;
        }
        Some(cur)
    }

    /// Get a mutable child by path, creating intermediate nodes as needed.
    pub fn get_mut(&mut self, path: &str) -> &mut RawConfig {
        let mut cur = self;
        for part in path.split('/') {
            cur = cur.sub_mut(part);
        }
        cur
    }

    /// Set the value at `path` (creating intermediate nodes as needed).
    pub fn set(&mut self, path: &str, value: impl Into<String>) {
        self.get_mut(path).set_value(value);
    }

    /// Get (or create) a direct child by name.
    pub fn sub_mut(&mut self, name: &str) -> &mut RawConfig {
        if let Some(&i) = self.index.get(name) {
            return &mut self.sub_items[i];
        }
        let child = RawConfig::new(name.to_owned());
        self.sub_items.push(child);
        let i = self.sub_items.len() - 1;
        self.index.insert(name.to_owned(), i);
        &mut self.sub_items[i]
    }
}

/// `config["A"]["B"]["C"]` style access, auto-creating nodes.
impl<'a> std::ops::Index<&'a str> for RawConfig {
    type Output = RawConfig;
    fn index(&self, key: &'a str) -> &RawConfig {
        self.child(key)
            .unwrap_or_else(|| panic!("RawConfig: no sub item `{key}`"))
    }
}

/// `config["A"]["B"]["C"] = value` style assignment, auto-creating nodes.
impl<'a> std::ops::IndexMut<&'a str> for RawConfig {
    fn index_mut(&mut self, key: &'a str) -> &mut RawConfig {
        self.sub_mut(key)
    }
}

/// Escape a value for INI output, replicating `fcitx::stringutils::escapeForValue`.
///
/// Values containing any of `\f \r \t \v " \ \n` (or a plain space) are
/// wrapped in double quotes with the special characters backslash-escaped.
fn escape_for_value(value: &str) -> String {
    let need_escape = value
        .chars()
        .any(|c| matches!(c, '\u{c}' | '\r' | '\t' | '\u{b}' | ' ' | '"' | '\\' | '\n'));
    let mut out = String::with_capacity(value.len() + 2);
    if need_escape {
        out.push('"');
    }
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\u{c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{b}' => out.push_str("\\v"),
            _ => out.push(c),
        }
    }
    if need_escape {
        out.push('"');
    }
    out
}

/// Serialize `config` to INI, replicating `fcitx::writeAsIni` semantics.
///
/// Output layout (matching fcitx5's `writeAsIni`):
/// - Every node that has sub items and produces leaf values is written as a
///   `[Path/To/Node]` section followed by its direct leaf children as
///   `Key=Value` lines, then a blank line.
/// - Children that are pure containers (have sub items and no value) are not
///   emitted in the parent section; they produce their own sections.
/// - Traversal is depth-first in insertion order.
pub fn write_ini(config: &RawConfig, out: &mut String) {
    write_node(config, "", out);
}

fn write_node(config: &RawConfig, path: &str, out: &mut String) {
    if config.has_sub_items() {
        let mut values = String::new();
        for child in config.sub_items() {
            if child.has_sub_items() && child.value().is_empty() {
                continue;
            }
            let _ = writeln!(values, "{}={}", child.name(), escape_for_value(child.value()));
        }
        if !values.is_empty() {
            if !path.is_empty() {
                let _ = writeln!(out, "[{}]", path);
            }
            out.push_str(&values);
            out.push('\n');
        }
    }
    for child in config.sub_items() {
        let child_path = if path.is_empty() {
            child.name().to_owned()
        } else {
            format!("{path}/{}", child.name())
        };
        write_node(child, &child_path, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape() {
        assert_eq!(escape_for_value("plain"), "plain");
        assert_eq!(escape_for_value("has space"), "\"has space\"");
        assert_eq!(escape_for_value("a\"b"), "\"a\\\"b\"");
        assert_eq!(escape_for_value("line\nbreak"), "\"line\\nbreak\"");
        assert_eq!(escape_for_value("#fcfcfc"), "#fcfcfc");
    }

    #[test]
    fn test_set_and_get() {
        let mut config = RawConfig::new("");
        config.set("Metadata/Name", "Plasma");
        assert_eq!(config.get("Metadata/Name").unwrap().value(), "Plasma");
        assert_eq!(config["Metadata"]["Name"].value(), "Plasma");
    }

    #[test]
    fn test_serialize() {
        let mut config = RawConfig::new("");
        config.set("Metadata/Name", "Plasma");
        config.set("InputPanel/NormalColor", "#fcfcfc");
        config.set("InputPanel/PageButtonAlignment", "Last Candidate");
        config.set("InputPanel/Background/Image", "panel.png");
        config.set("InputPanel/Background/Margin/Left", "14");

        let mut out = String::new();
        write_ini(&config, &mut out);
        let expected = "[Metadata]\nName=Plasma\n\n[InputPanel]\nNormalColor=#fcfcfc\nPageButtonAlignment=\"Last Candidate\"\n\n[InputPanel/Background]\nImage=panel.png\n\n[InputPanel/Background/Margin]\nLeft=14\n\n";
        assert_eq!(out, expected);
    }
}
