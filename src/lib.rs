//! Perfuse's media model and compiled-in media source plugins.

pub mod media;
pub mod plugins;

mod cache;
mod http;

pub fn trim_artwork_cache() {
    if let Some(dir) = cache::dir() {
        cache::trim(&dir);
    }
}
