use std::{collections::HashMap, sync::Arc};

use bytes::Bytes;
use gpui_kit::{Image, ImageFormat, ImageSource};

use crate::library::CoverId;

pub const SMALL_COVER_PX: u32 = 96;

pub enum Cover {
    Loading,
    Loaded(ImageSource),
    Failed,
}

pub fn image_format(bytes: &[u8]) -> ImageFormat {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        ImageFormat::Png
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        ImageFormat::Jpeg
    } else if bytes.starts_with(b"RIFF") {
        ImageFormat::Webp
    } else {
        ImageFormat::Jpeg
    }
}

pub fn decode(bytes: &Bytes) -> Cover {
    Cover::Loaded(decode_image(bytes))
}

pub fn decode_image(bytes: &Bytes) -> ImageSource {
    Arc::new(Image::from_bytes(image_format(bytes), bytes.to_vec())).into()
}

#[derive(Default)]
pub struct CoverCache {
    cache: HashMap<CoverId, Cover>,
}

impl CoverCache {
    pub fn get(&self, id: &CoverId) -> Option<&Cover> {
        self.cache.get(id)
    }

    // returns `true` if the caller should start the fetch
    pub fn start_fetch(&mut self, id: &CoverId) -> bool {
        if self.cache.contains_key(id) {
            return false;
        }

        self.cache.insert(id.clone(), Cover::Loading);
        true
    }

    /// Returns `true` if the result was still wanted (caller should notify).
    pub fn end_fetch(&mut self, id: CoverId, cover: Cover) -> bool {
        if !matches!(self.cache.get(&id), Some(Cover::Loading)) {
            return false;
        }
        self.cache.insert(id, cover);
        true
    }

    pub fn clear(&mut self) {
        self.cache.clear();
    }
}
