//! On Windows (MSVC), embeds the app icon in the .exe for Explorer and the taskbar:
//! it is drawn with the same code as the tray icon (src/icon.rs) and written as a
//! compiled resource (.res, src/winres.rs), which the MSVC linker accepts as is.
//! No external tools.

#[path = "src/icon.rs"]
mod icon;
#[path = "src/winres.rs"]
mod winres;

use std::path::Path;

/// Icon sizes: the small ones as bitmaps; the 256 one (large icons view) as compressed PNG,
/// so the executable doesn't bloat.
const DIB_SIZES: [usize; 6] = [16, 20, 24, 32, 48, 64];
const PNG_SIZE: usize = 256;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/icon.rs");
    println!("cargo:rerun-if-changed=src/winres.rs");
    let windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if !windows {
        return;
    }
    if !msvc {
        // Only the MSVC linker takes the .res directly
        println!("cargo:warning=Nogenuity.exe gets no icon: it is only embedded with the MSVC target");
        return;
    }
    let images: Vec<(usize, Vec<u8>)> = DIB_SIZES
        .iter()
        .map(|&n| (n, winres::dib(n, &icon::pixels(n))))
        .chain(std::iter::once((PNG_SIZE, winres::png(PNG_SIZE, &icon::pixels(PNG_SIZE)))))
        .collect();
    let out = std::env::var("OUT_DIR").expect("OUT_DIR");
    let path = Path::new(&out).join("nogenuity.res");
    std::fs::write(&path, winres::res(&images)).expect("could not write the icon resource");
    println!("cargo:rustc-link-arg-bins={}", path.display());
}
