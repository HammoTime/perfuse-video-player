//! Embeds the Perfuse icon in the Windows executable for Explorer, pinned
//! shortcuts and the taskbar. The .ico is rasterised from assets/brand at
//! build time, so the SVGs remain the only source.

use resvg::{tiny_skia, usvg};
use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=assets/brand");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR"));
    let ico = out.join("perfuse.ico");
    fs::write(&ico, icon()).expect("write perfuse.ico");
    let rc = out.join("perfuse.rc");
    // Resource 1 is also loaded at runtime for the large taskbar icon.
    let path = ico.display().to_string().replace('\\', "/");
    fs::write(&rc, format!("1 ICON \"{path}\"\n")).expect("write perfuse.rc");
    embed_resource::compile(&rc, embed_resource::NONE)
        .manifest_optional()
        .expect("compile the icon resource");
}

/// A PNG-compressed .ico. Small sizes use the variant drawn for 32px.
fn icon() -> Vec<u8> {
    let small = include_str!("assets/brand/perfuse-app-icon-32.svg");
    let large = include_str!("assets/brand/perfuse-app-icon.svg");
    let images: Vec<(u32, Vec<u8>)> = [16, 20, 24, 32, 40, 48, 64, 128, 256]
        .into_iter()
        .map(|size| (size, png(if size <= 32 { small } else { large }, size)))
        .collect();
    let mut ico = Vec::new();
    ico.extend([0, 0, 1, 0]);
    ico.extend((images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (size, png) in &images {
        // A 256px entry is written as 0 in the one-byte dimensions.
        let dimension = (*size % 256) as u8;
        ico.extend([dimension, dimension, 0, 0]);
        ico.extend(1u16.to_le_bytes());
        ico.extend(32u16.to_le_bytes());
        ico.extend((png.len() as u32).to_le_bytes());
        ico.extend(offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for (_, png) in images {
        ico.extend(png);
    }
    ico
}

fn png(svg: &str, size: u32) -> Vec<u8> {
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).expect("parse icon SVG");
    let mut pixmap = tiny_skia::Pixmap::new(size, size).expect("icon pixmap");
    let scale = size as f32 / tree.size().width();
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap.encode_png().expect("encode icon PNG")
}
