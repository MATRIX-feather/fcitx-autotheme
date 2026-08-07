//! Minimal fcitx `RawConfig`-style tree and INI serialization.
//!
//! Mirrors `fcitx::RawConfig` + `fcitx::safeSaveAsIni`: nested paths are
//! serialized as `[Parent/Child]` sections, values containing spaces are
//! quoted, and the tree is written depth-first in insertion order.

use std::collections::HashMap;

/// A node of the config tree.
#[derive(Debug, Default)]
struct Node {
    values: Vec<(String, String)>,
    children: HashMap<String, Self>,
    order: Vec<String>,
}

impl Node {
    fn child_mut(&mut self, name: &str) -> &mut Self {
        if !self.children.contains_key(name) {
            self.order.push(name.to_owned());
        }
        self.children.entry(name.to_owned()).or_default()
    }
}

/// A config tree with `/`-separated section and key paths.
#[derive(Debug, Default)]
pub struct Config {
    root: Node,
}

impl Config {
    /// Create an empty config.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a value at a `/`-separated path; the last segment is the key.
    pub fn set(&mut self, path: &str, value: &str) {
        let mut parts = path.split('/');
        let Some(key) = parts.next_back() else {
            return;
        };
        let node = parts.fold(&mut self.root, |node, part| node.child_mut(part));
        let values = &mut node.values;
        if let Some((_, v)) = values.iter_mut().find(|(k, _)| k == key) {
            value.clone_into(v);
        } else {
            values.push((key.to_owned(), value.to_owned()));
        }
    }

    /// Serialize in fcitx INI format.
    #[must_use]
    pub fn serialize(&self) -> String {
        let mut out = String::new();
        write_node(&mut out, &self.root, "");
        out
    }
}

/// Recursively serialize a node: its values under `[path]`, then children.
fn write_node(out: &mut String, node: &Node, path: &str) {
    if !node.values.is_empty() {
        if !path.is_empty() {
            out.push('[');
            out.push_str(path);
            out.push_str("]\n");
        }
        for (key, value) in &node.values {
            out.push_str(key);
            out.push('=');
            out.push_str(&quote(value));
            out.push('\n');
        }
        out.push('\n');
    }
    for name in &node.order {
        if let Some(child) = node.children.get(name) {
            let child_path = if path.is_empty() {
                name.clone()
            } else {
                format!("{path}/{name}")
            };
            write_node(out, child, &child_path);
        }
    }
}

/// Quote values containing whitespace or quote characters, like fcitx.
fn quote(value: &str) -> String {
    if value.contains(char::is_whitespace) || value.contains('"') {
        format!("\"{}\"", value.replace('"', "\\\""))
    } else {
        value.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_nested_sections() {
        let mut config = Config::new();
        config.set("InputPanel/NormalColor", "#aabbcc");
        config.set("InputPanel/PageButtonAlignment", "Last Candidate");
        config.set("InputPanel/Background/Image", "panel.png");
        config.set("Menu/Spacing", "2.000000");
        let text = config.serialize();
        assert!(text.contains("[InputPanel]\nNormalColor=#aabbcc\n"));
        assert!(text.contains("PageButtonAlignment=\"Last Candidate\"\n"));
        assert!(text.contains("[InputPanel/Background]\nImage=panel.png\n"));
        assert!(text.contains("[Menu]\nSpacing=2.000000\n"));
    }

    #[test]
    fn insertion_order_preserved() {
        let mut config = Config::new();
        config.set("B/x", "1");
        config.set("A/y", "2");
        let text = config.serialize();
        assert!(text.find("[B]").expect("B") < text.find("[A]").expect("A"));
    }

    #[test]
    fn overwrite_keeps_position() {
        let mut config = Config::new();
        config.set("S/a", "1");
        config.set("S/b", "2");
        config.set("S/a", "3");
        let text = config.serialize();
        assert!(text.find("a=3").expect("a") < text.find("b=2").expect("b"));
    }
}
