//! Formats d'image reconnus (d'après l'extension du fichier).

use std::path::Path;

/// Extensions (en minuscules) des formats que l'on sait décoder.
pub const EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "jpe", "jfif", "png", "webp", "gif", "bmp", "dib", "tif", "tiff", "ico", "tga",
    "pbm", "pgm", "ppm", "pnm",
];

/// `true` si le chemin a une extension d'image connue (insensible à la casse).
pub fn is_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            let e = e.to_ascii_lowercase();
            EXTENSIONS.contains(&e.as_str())
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_common_formats_case_insensitively() {
        for name in [
            "a.jpg", "a.JPG", "a.jpeg", "a.Png", "a.webp", "a.WEBP", "a.gif", "a.bmp", "a.tif",
            "a.TIFF", "dir.x/b.png",
        ] {
            assert!(is_image_path(Path::new(name)), "{name} devrait être reconnu");
        }
    }

    #[test]
    fn rejects_non_images() {
        for name in ["a.txt", "a", "a.jpg.txt", ".jpg", "a.pdf", "archive.zip", "a.json"] {
            assert!(!is_image_path(Path::new(name)), "{name} ne devrait pas être reconnu");
        }
    }
}
