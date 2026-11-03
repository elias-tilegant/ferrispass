use std::borrow::Cow;

use anyhow::anyhow;
use gpui::{AssetSource, Result, SharedString};
use gpui_kit_assets::Assets as ComponentAssets;
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "assets"]
#[include = "icons/**/*.svg"]
#[include = "fonts/**/*.ttf"]
pub struct ProjectAssets;

pub struct AppAssets {
    component: ComponentAssets,
}

impl AppAssets {
    pub fn new() -> Self {
        Self {
            component: ComponentAssets,
        }
    }
}

impl Default for AppAssets {
    fn default() -> Self {
        Self::new()
    }
}

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path.is_empty() {
            return Ok(None);
        }

        if let Some(file) = ProjectAssets::get(path) {
            return Ok(Some(file.data));
        }

        match self.component.load(path) {
            Ok(Some(data)) => Ok(Some(data)),
            Ok(None) => Err(anyhow!("could not find asset at path \"{path}\"")),
            Err(error) => Err(error),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut entries: Vec<SharedString> = ProjectAssets::iter()
            .filter_map(|p| p.starts_with(path).then(|| p.into()))
            .collect();
        if let Ok(more) = self.component.list(path) {
            entries.extend(more);
        }
        Ok(entries)
    }
}

/// The fonts the app bundles. Atkinson Hyperlegible Mono is for secrets in the
/// new screens; the old screens still use JetBrains Mono. Both come as
/// separate static files per weight: GPUI gets only one weight out of a
/// variable font.
const FONTS: [&str; 4] = [
    "fonts/AtkinsonHyperlegibleMono-Regular.ttf",
    "fonts/AtkinsonHyperlegibleMono-Medium.ttf",
    "fonts/JetBrainsMono-Regular.ttf",
    "fonts/JetBrainsMono-Medium.ttf",
];

pub fn font_bytes() -> Vec<Cow<'static, [u8]>> {
    FONTS
        .into_iter()
        .filter_map(|path| ProjectAssets::get(path).map(|file| file.data))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A font missing from the embed would not fail the build, only drop out
    /// of `font_bytes`.
    #[test]
    fn every_bundled_font_is_embedded() {
        for path in FONTS {
            assert!(ProjectAssets::get(path).is_some(), "{path} is not embedded");
        }
        assert_eq!(font_bytes().len(), FONTS.len());
    }
}
