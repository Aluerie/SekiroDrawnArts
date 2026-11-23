//! Combat art images for the wheel, loaded from PNG files named after each art's uid.

use std::{collections::HashMap, path::Path};

use anyhow::Context;
use hudhook::{RenderContext, imgui::TextureId};

use crate::{arts, game::Uid};

/// GPU textures of the combat art images that were found.
#[derive(Default)]
pub struct Icons(HashMap<Uid, TextureId>);

impl Icons {
    /// Uploads `<dir>/<uid>.png` for every combat art that has one. Arts without an image, or
    /// with one that can't be read, are logged and left out.
    pub fn load(dir: &Path, render_context: &mut dyn RenderContext) -> Self {
        let mut icons = HashMap::new();
        for art in arts::ALL {
            let path = dir.join(format!("{}.png", art.uid.0));
            if !path.exists() {
                continue;
            }
            let texture = decode(&path).and_then(|(rgba, width, height)| {
                render_context
                    .load_texture(&rgba, width, height)
                    .map_err(|e| anyhow::anyhow!("can't upload the texture: {e:?}"))
            });
            match texture {
                Ok(texture) => {
                    icons.insert(art.uid, texture);
                }
                Err(e) => log::warn!("skipping the image for {}: {e:#}", art.name),
            }
        }
        log::info!("loaded {} combat art images from {}", icons.len(), dir.display());
        Self(icons)
    }

    pub fn get(&self, uid: Uid) -> Option<TextureId> {
        self.0.get(&uid).copied()
    }
}

/// Reads a PNG file as tightly packed RGBA8 pixels.
fn decode(path: &Path) -> anyhow::Result<(Vec<u8>, u32, u32)> {
    let image = image::open(path).with_context(|| format!("can't read {}", path.display()))?.into_rgba8();
    let (width, height) = image.dimensions();
    Ok((image.into_raw(), width, height))
}
