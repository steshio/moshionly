//! Writes Moshionly's mark as the PNG images of a macOS `.iconset` folder,
//! for `iconutil` to pack into the app's icon. `scripts/release-mac.sh`
//! runs it while it assembles `Moshionly.app`, so the Finder icon and the
//! window icon come from the same drawing.
//!
//! ```text
//! cargo run --release -p moshionly-app --example app_icon -- AppIcon.iconset
//! iconutil -c icns -o AppIcon.icns AppIcon.iconset
//! ```
//!
//! The folder is created if needed; images already in it are replaced.

use std::path::PathBuf;

use image::{ImageBuffer, Rgba};
use moshionly_app::theme::brand::{APP_ICON_SET, app_icon_rgba};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let folder: PathBuf = std::env::args_os()
        .nth(1)
        .ok_or("give the folder to write the icon images into, for example AppIcon.iconset")?
        .into();
    std::fs::create_dir_all(&folder)?;
    for (name, side) in APP_ICON_SET {
        let image = ImageBuffer::<Rgba<u8>, _>::from_raw(side, side, app_icon_rgba(side))
            .ok_or("the icon image has the wrong number of pixels")?;
        let path = folder.join(name);
        image
            .save(&path)
            .map_err(|error| format!("{} could not be written: {error}", path.display()))?;
    }
    println!(
        "wrote {} icon images to {}",
        APP_ICON_SET.len(),
        folder.display()
    );
    Ok(())
}
