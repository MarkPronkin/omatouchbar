//! The Omarchy menu tree, read from Omarchy's own JSONC definition.
//!
//! IDs are object keys and dotted IDs imply hierarchy (`trigger.share.file`
//! belongs under `trigger.share`). An entry with `action` runs a command,
//! one with `target` links to another submenu, anything else is a submenu.
//! The user's `~/.config/omarchy/extensions/omarchy-menu.jsonc` may add
//! entries or override fields of existing ones.

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value};

use crate::paths;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MenuItem {
    pub id: String,
    pub icon: String,
    /// The icon uses the Omarchy logo font.
    pub logo_icon: bool,
    pub label: String,
    pub title: Option<String>,
    pub action: Option<String>,
    pub target: Option<String>,
    pub provider: Option<String>,
    pub when: Option<String>,
    pub checked: Option<String>,
    pub disabled: Option<String>,
    /// Optional sibling id to place this entry right after/before.
    pub after: Option<String>,
    pub before: Option<String>,
    /// Provider rows: the row is the current value.
    pub current: bool,
}

impl MenuItem {
    pub fn parent(&self) -> &str {
        parent_of(&self.id)
    }

    /// Static menus and links get their content from the named parent.
    /// Providers populate their own rows asynchronously.
    fn static_menu(&self) -> Option<&str> {
        (self.action.is_none() && self.provider.is_none())
            .then(|| self.target.as_deref().unwrap_or(&self.id))
    }
}

pub fn parent_of(id: &str) -> &str {
    id.rfind('.').map(|i| &id[..i]).unwrap_or("")
}

#[derive(Clone, Debug, Default)]
pub struct MenuTree {
    items: Vec<MenuItem>,
    index: HashMap<String, usize>,
    children: HashMap<String, Vec<usize>>,
}

/// Strips `//` and `/* */` comments and trailing commas, leaving strings
/// untouched, so the result is plain JSON.
pub fn strip_jsonc(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    let mut in_string = false;
    while i < bytes.len() {
        let c = bytes[i];
        if in_string {
            if c == b'\\' && i + 1 < bytes.len() {
                out.push_str(&input[i..i + 2]);
                i += 2;
                continue;
            }
            if c == b'"' {
                in_string = false;
            }
            let ch_len = utf8_len(c);
            out.push_str(&input[i..i + ch_len]);
            i += ch_len;
            continue;
        }
        match c {
            b'"' => {
                in_string = true;
                out.push('"');
                i += 1;
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i += 2;
            }
            b',' => {
                let mut j = i + 1;
                while j < bytes.len() && (bytes[j] as char).is_ascii_whitespace() {
                    j += 1;
                }
                // Also look past comments for the trailing-comma check.
                let rest = strip_leading_comments(&input[j.min(input.len())..]);
                if !matches!(rest.as_bytes().first(), Some(b'}') | Some(b']')) {
                    out.push(',');
                }
                i += 1;
            }
            _ => {
                let ch_len = utf8_len(c);
                out.push_str(&input[i..i + ch_len]);
                i += ch_len;
            }
        }
    }
    out
}

fn strip_leading_comments(mut s: &str) -> &str {
    loop {
        s = s.trim_start();
        if let Some(rest) = s.strip_prefix("//") {
            s = rest.find('\n').map(|i| &rest[i..]).unwrap_or("");
        } else if let Some(rest) = s.strip_prefix("/*") {
            s = rest.find("*/").map(|i| &rest[i + 2..]).unwrap_or("");
        } else {
            return s;
        }
    }
}

fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        _ => 4,
    }
}

fn field(map: &Map<String, Value>, name: &str) -> Option<String> {
    map.get(name).and_then(Value::as_str).map(str::to_string)
}

impl MenuTree {
    /// Loads Omarchy's default menu plus the user's extension.
    pub fn load() -> MenuTree {
        let mut tree = MenuTree::default();
        let root = paths::omarchy_root();
        for path in [
            root.join("default/omarchy/omarchy-menu.jsonc"),
            paths::config_home().join("omarchy/extensions/omarchy-menu.jsonc"),
        ] {
            if let Ok(text) = std::fs::read_to_string(&path)
                && let Err(e) = tree.merge_jsonc(&text)
            {
                log::warn!("menu {}: {e}", path.display());
            }
        }
        tree.rebuild();
        tree
    }

    pub fn from_jsonc(text: &str) -> Result<MenuTree, String> {
        let mut tree = MenuTree::default();
        tree.merge_jsonc(text)?;
        tree.rebuild();
        Ok(tree)
    }

    fn merge_jsonc(&mut self, text: &str) -> Result<(), String> {
        let value: Value = serde_json::from_str(&strip_jsonc(text)).map_err(|e| e.to_string())?;
        let Value::Object(entries) = value else {
            return Err("menu is not an object".into());
        };
        for (id, entry) in entries {
            let Value::Object(map) = entry else { continue };
            let existing = self.items.iter().position(|item| item.id == id);
            let mut item = existing
                .map(|i| self.items[i].clone())
                .unwrap_or_else(|| MenuItem {
                    id: id.clone(),
                    ..Default::default()
                });
            if let Some(v) = field(&map, "icon") {
                item.icon = v;
            }
            if let Some(v) = field(&map, "iconFont") {
                item.logo_icon = v == "omarchy";
            }
            if let Some(v) = field(&map, "label") {
                item.label = v;
            }
            if let Some(v) = field(&map, "title") {
                item.title = Some(v);
            }
            if let Some(v) = field(&map, "action") {
                item.action = Some(v);
            }
            if let Some(v) = field(&map, "target") {
                item.target = Some(v);
            }
            if let Some(v) = field(&map, "provider") {
                item.provider = Some(v);
            }
            if let Some(v) = field(&map, "when") {
                item.when = Some(v);
            }
            if let Some(v) = field(&map, "checked") {
                item.checked = Some(v);
            }
            if let Some(v) = field(&map, "disabled") {
                item.disabled = Some(v);
            }
            if let Some(v) = field(&map, "after") {
                item.after = Some(v);
            }
            if let Some(v) = field(&map, "before") {
                item.before = Some(v);
            }
            if item.label.is_empty() {
                item.label = id.rsplit('.').next().unwrap_or(&id).replace('-', " ");
            }
            match existing {
                Some(i) => self.items[i] = item,
                None => self.items.push(item),
            }
        }
        Ok(())
    }

    fn rebuild(&mut self) {
        let ids: Vec<String> = self.items.iter().map(|i| i.id.clone()).collect();
        for id in ids {
            let Some(cur) = self.items.iter().position(|i| i.id == id) else {
                continue;
            };
            let (after, before) = (
                self.items[cur].after.clone(),
                self.items[cur].before.clone(),
            );
            let target = if let Some(ref anchor) = after {
                if anchor == &id {
                    continue;
                }
                self.items
                    .iter()
                    .position(|i| i.id == *anchor)
                    .map(|pos| (pos, true))
            } else if let Some(ref anchor) = before {
                if anchor == &id {
                    continue;
                }
                self.items
                    .iter()
                    .position(|i| i.id == *anchor)
                    .map(|pos| (pos, false))
            } else {
                None
            };
            let Some((target_pos, is_after)) = target else {
                continue;
            };
            let item = self.items.remove(cur);
            let adjusted = if cur < target_pos {
                target_pos - 1
            } else {
                target_pos
            };
            let insert_at = if is_after { adjusted + 1 } else { adjusted };
            self.items.insert(insert_at, item);
        }
        self.index = self
            .items
            .iter()
            .enumerate()
            .map(|(i, item)| (item.id.clone(), i))
            .collect();
        self.children.clear();
        for (i, item) in self.items.iter().enumerate() {
            self.children
                .entry(item.parent().to_string())
                .or_default()
                .push(i);
        }
    }

    pub fn get(&self, id: &str) -> Option<&MenuItem> {
        self.index.get(id).map(|&i| &self.items[i])
    }

    pub fn children(&self, parent: &str) -> impl Iterator<Item = &MenuItem> {
        self.children
            .get(parent)
            .into_iter()
            .flatten()
            .map(|&i| &self.items[i])
    }

    pub fn has_children(&self, id: &str) -> bool {
        self.children.get(id).is_some_and(|c| !c.is_empty())
    }

    /// Whether tapping the item opens a submenu on the Touch Bar.
    pub fn is_submenu(&self, item: &MenuItem) -> bool {
        item.action.is_none()
            && (item.target.is_some() || self.has_children(&item.id) || item.provider.is_some())
    }

    /// As in Omarchy, a static submenu needs at least one visible descendant.
    pub(crate) fn is_visible(&self, item: &MenuItem, when: impl Fn(&str) -> bool) -> bool {
        let mut pending = vec![item];
        let mut expanded = HashSet::new();
        while let Some(item) = pending.pop() {
            if item.when.is_some() && !when(&item.id) {
                continue;
            }
            let Some(id) = item.static_menu() else {
                return true;
            };
            if expanded.insert(id) {
                pending.extend(self.children(id));
            }
        }
        false
    }

    /// Only probe descendants when the parent's visibility depends on them.
    /// A submenu containing an unconditional action can always be entered.
    pub(crate) fn visibility_guards(&self, parent: &str) -> Vec<(&str, &str)> {
        let mut guarded = Vec::new();
        let mut seen = HashSet::new();
        let mut pending: Vec<_> = self.children(parent).collect();
        while let Some(item) = pending.pop() {
            if !seen.insert(&item.id) {
                continue;
            }
            if let Some(expr) = &item.when {
                guarded.push((item.id.as_str(), expr.as_str()));
            }
            if let Some(id) = item.static_menu()
                && !self
                    .children(id)
                    .any(|child| self.is_visible(child, |_| false))
            {
                pending.extend(self.children(id));
            }
        }
        guarded
    }

    pub fn title(&self, id: &str) -> String {
        match self.get(id) {
            Some(item) => item.title.clone().unwrap_or_else(|| item.label.clone()),
            None => "Menu".into(),
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// Built-in provider scripts (mirroring the Omarchy shell's `providers`).
/// Each prints `label\tvalue\tcurrent` rows.
pub fn provider_script(name: &str) -> Option<(&'static str, &'static str)> {
    // (script, action prefix)
    match name {
        "fonts" => Some((
            "current=$(omarchy-font-current 2>/dev/null); omarchy-font-list 2>/dev/null | while read -r f; do [[ -z $f ]] && continue; printf '%s\\t%s\\t%s\\n' \"$f\" \"$f\" \"$current\"; done",
            "omarchy-font-set",
        )),
        "power-profiles" => Some((
            "current=$(powerprofilesctl get 2>/dev/null); omarchy-powerprofiles-list 2>/dev/null | while read -r p; do [[ -z $p ]] && continue; printf '%s\\t%s\\t%s\\n' \"$p\" \"$p\" \"$current\"; done",
            "omarchy-powerprofiles-set autodetect",
        )),
        "kernels" => Some((
            "current=$(omarchy-kernel get 2>/dev/null); omarchy-kernel list 2>/dev/null | while read -r k; do [[ -z $k ]] && continue; printf '%s\\t%s\\t%s\\n' \"$k\" \"$k\" \"$current\"; done",
            "omarchy-kernel set",
        )),
        "languages" => Some((
            "current=$(omarchy-language 2>/dev/null); omarchy-language list 2>/dev/null | while IFS=$'\\t' read -r name code english; do [[ -z $code ]] && continue; printf '%s\\t%s\\t%s\\n' \"$name\" \"$code\" \"$current\"; done",
            "omarchy-language",
        )),
        _ => None,
    }
}

/// Example choices for provider menus in the interactive preview and snapshots.
pub fn sample_provider_rows(provider: &str) -> Vec<(String, String, bool)> {
    let choices: &[(&str, &str)] = match provider {
        "fonts" => &[
            ("JetBrainsMono Nerd Font", "JetBrainsMono Nerd Font"),
            ("Cascadia Code", "Cascadia Code"),
        ],
        "power-profiles" => &[
            ("balanced", "balanced"),
            ("performance", "performance"),
            ("power-saver", "power-saver"),
        ],
        "kernels" => &[("linux", "linux"), ("linux-lts", "linux-lts")],
        "languages" => &[
            ("English", "en_US"),
            ("Deutsch", "de_DE"),
            ("Français", "fr_FR"),
        ],
        _ => &[],
    };
    choices
        .iter()
        .enumerate()
        .map(|(i, &(label, value))| (label.into(), value.into(), i == 0))
        .collect()
}

/// Quotes a value for bash.
pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_comments_and_trailing_commas() {
        let text = r#"{
            // comment with "quotes"
            "a": {"label": "A // not a comment", "action": "x",}, /* block */
            "a.b": {"label": "B",},
        }"#;
        let tree = MenuTree::from_jsonc(text).unwrap();
        assert_eq!(tree.get("a").unwrap().label, "A // not a comment");
        assert_eq!(tree.children("a").count(), 1);
        assert_eq!(tree.children("").count(), 1);
    }

    #[test]
    fn extension_overrides_fields() {
        let mut tree = MenuTree::default();
        tree.merge_jsonc(r#"{"about": {"icon": "i", "label": "About", "action": "x"}}"#)
            .unwrap();
        tree.merge_jsonc(r#"{"about": {"action": "y"}, "me": {"label": "Me"}, "me.a": {"label":"A","action":"z"}}"#).unwrap();
        tree.rebuild();
        let about = tree.get("about").unwrap();
        assert_eq!(about.icon, "i");
        assert_eq!(about.action.as_deref(), Some("y"));
        assert!(tree.is_submenu(tree.get("me").unwrap()));
    }

    #[test]
    fn positions_item_after_or_before_sibling() {
        let mut tree = MenuTree::default();
        tree.merge_jsonc(
            r#"{
                "setup": {"label": "Setup"},
                "setup.input": {"label": "Input", "action": "a"},
                "setup.typing": {"label": "Typing"},
                "setup.language": {"label": "Language", "action": "b"}
            }"#,
        )
        .unwrap();
        tree.merge_jsonc(
            r#"{
                "setup.touch-bar": {"label": "Touch Bar", "after": "setup.typing", "action": "c"},
                "setup.early": {"label": "Early", "before": "setup.input", "action": "d"}
            }"#,
        )
        .unwrap();
        tree.rebuild();
        let labels: Vec<&str> = tree.children("setup").map(|i| i.label.as_str()).collect();
        assert_eq!(
            labels,
            ["Early", "Input", "Typing", "Touch Bar", "Language"]
        );
    }

    #[test]
    fn loads_installed_omarchy_menu_if_present() {
        let path = paths::omarchy_root().join("default/omarchy/omarchy-menu.jsonc");
        if let Ok(text) = std::fs::read_to_string(path) {
            let tree = MenuTree::from_jsonc(&text).unwrap();
            assert!(tree.children("").count() >= 5);
            assert!(tree.get("system.lock").is_some());
        }
    }
}
