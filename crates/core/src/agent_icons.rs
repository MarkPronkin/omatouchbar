//! Provider logos come from the same installed assets as Omarchy's Agents panel.

use std::{
    collections::HashMap,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::SystemTime,
};

use crate::{color::Rgba, paths, state::IconImage};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AgentIcons {
    regular: Option<IconImage>,
    light: Option<IconImage>,
}

impl AgentIcons {
    pub fn on_surface(&self, surface: Rgba) -> Option<&IconImage> {
        // Omarchy chooses the light-surface asset at relative luminance 0.5.
        let linear = |v: f32| {
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        let luminance =
            0.2126 * linear(surface.r) + 0.7152 * linear(surface.g) + 0.0722 * linear(surface.b);
        if luminance >= 0.5 {
            self.light.as_ref().or(self.regular.as_ref())
        } else {
            self.regular.as_ref()
        }
    }
}

#[derive(Default)]
pub struct AgentIconCache {
    images: HashMap<PathBuf, (SystemTime, IconImage)>,
}

impl AgentIconCache {
    pub fn load(&mut self, id: &str) -> AgentIcons {
        self.load_from(
            &paths::omarchy_root().join("shell/plugins/agents/assets"),
            id,
        )
    }

    fn load_from(&mut self, dir: &Path, id: &str) -> AgentIcons {
        if Path::new(id).file_name() != Some(OsStr::new(id)) {
            return AgentIcons::default();
        }
        AgentIcons {
            regular: self.image(dir.join(format!("{id}.svg"))),
            light: self.image(dir.join(format!("{id}-light.svg"))),
        }
    }

    fn image(&mut self, path: PathBuf) -> Option<IconImage> {
        let modified = fs::metadata(&path).ok()?.modified().ok()?;
        if let Some((stamp, image)) = self.images.get(&path)
            && *stamp == modified
        {
            return Some(image.clone());
        }
        let output = Command::new("rsvg-convert")
            .args(["--width=64", "--height=64", "--keep-aspect-ratio", "--"])
            .arg(&path)
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let image = IconImage(tiny_skia::Pixmap::decode_png(&output.stdout).ok()?.into());
        self.images.insert(path, (modified, image.clone()));
        Some(image)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_provider_uses_installed_artwork_for_the_surface() {
        let dir = std::env::temp_dir().join(format!("omatouchbar-icons-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let mut cache = AgentIconCache::default();
        let svg = |fill: &str| {
            format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 4 4"><rect width="4" height="4" fill="{fill}"/></svg>"#
            )
        };
        let id = "unlisted-provider";
        assert!(cache.load_from(&dir, id).on_surface(Rgba::WHITE).is_none());
        fs::write(dir.join(format!("{id}.svg")), svg("orange")).unwrap();
        let icons = cache.load_from(&dir, id);
        assert_eq!(icons.on_surface(Rgba::BLACK), icons.on_surface(Rgba::WHITE));
        fs::write(dir.join(format!("{id}-light.svg")), svg("blue")).unwrap();
        let icons = cache.load_from(&dir, id);
        assert_ne!(
            icons.on_surface(Rgba::BLACK).unwrap(),
            icons.on_surface(Rgba::WHITE).unwrap()
        );
        assert!(
            cache
                .load_from(&dir, "../unlisted-provider")
                .regular
                .is_none()
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
