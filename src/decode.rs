//! Décodage d'une image et réduction à la taille de l'écran.
//!
//! Réduire dès le décodage (dans un thread de fond) rend l'affichage quasi
//! gratuit : la texture envoyée au GPU a la taille de l'écran, pas celle du
//! capteur de l'appareil photo.

use fast_image_resize::images::Image as FirImage;
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};
use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageReader, RgbImage, RgbaImage};
use std::fs;
use std::io::Cursor;
use std::path::Path;

/// Image décodée, orientée selon l'EXIF et réduite pour tenir dans la taille demandée.
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    /// Dimensions de l'image d'origine (après orientation EXIF).
    pub orig_width: u32,
    pub orig_height: u32,
    /// Pixels RGBA 8 bits, alpha non prémultiplié, `width * height * 4` octets.
    pub rgba: Vec<u8>,
}

/// Taille de `(w, h)` réduite pour tenir dans `(max_w, max_h)`, sans jamais agrandir.
pub fn fit_size(w: u32, h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    if w == 0 || h == 0 || (w <= max_w && h <= max_h) {
        return (w, h);
    }
    let scale = (max_w as f64 / w as f64).min(max_h as f64 / h as f64);
    (((w as f64 * scale).round() as u32).max(1), ((h as f64 * scale).round() as u32).max(1))
}

fn swaps_axes(o: Orientation) -> bool {
    matches!(o, Orientation::Rotate90 | Orientation::Rotate270 | Orientation::Rotate90FlipH | Orientation::Rotate270FlipH)
}

/// Charge `path`. Le format est deviné d'après le contenu (pas seulement l'extension).
pub fn decode_fit(path: &Path, max_w: u32, max_h: u32) -> Result<Decoded, String> {
    // Lecture complète puis décodage en mémoire : le fichier n'est ouvert que
    // quelques millisecondes, ce qui évite de bloquer son déplacement (Windows).
    let bytes = fs::read(path).map_err(|e| format!("lecture impossible : {e}"))?;
    let (img, orientation) = if is_avif(&bytes) {
        (decode_avif(&bytes)?, Orientation::NoTransforms)
    } else {
        decode_with_image_crate(bytes, path)?
    };

    let (sw, sh) = (img.width(), img.height());
    let (ow, oh) = if swaps_axes(orientation) { (sh, sw) } else { (sw, sh) };
    let (tw, th) = fit_size(ow, oh, max_w.max(1), max_h.max(1));
    // Taille cible exprimée dans le repère de l'image non pivotée.
    let (rw, rh) = if swaps_axes(orientation) { (th, tw) } else { (tw, th) };

    let has_alpha = img.color().has_alpha();
    let (pt, buf) = if has_alpha { (PixelType::U8x4, img.into_rgba8().into_raw()) } else { (PixelType::U8x3, img.into_rgb8().into_raw()) };

    let buf = if (rw, rh) != (sw, sh) { resize(buf, sw, sh, rw, rh, pt)? } else { buf };

    // Orientation EXIF appliquée sur l'image déjà réduite (donc bon marché).
    let (buf, w, h) = orient(buf, rw, rh, has_alpha, orientation)?;
    let rgba = if has_alpha { buf } else { rgb_to_rgba(&buf) };
    Ok(Decoded { width: w, height: h, orig_width: ow, orig_height: oh, rgba })
}

/// Formats gérés par la crate `image` (JPEG, PNG, WebP…), avec l'orientation EXIF.
fn decode_with_image_crate(bytes: Vec<u8>, path: &Path) -> Result<(DynamicImage, Orientation), String> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| format!("format inconnu : {e}"))?;
    if reader.format().is_none() {
        // Formats sans signature (ex. TGA) : on se rabat sur l'extension.
        if let Ok(f) = image::ImageFormat::from_path(path) {
            reader.set_format(f);
        }
    }
    let mut decoder = reader.into_decoder().map_err(|e| format!("format non pris en charge : {e}"))?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let img = DynamicImage::from_decoder(decoder).map_err(|e| format!("décodage impossible : {e}"))?;
    Ok((img, orientation))
}

/// `true` si le contenu est un fichier AVIF (boîte `ftyp` de marque `avif`/`avis`).
pub fn is_avif(bytes: &[u8]) -> bool {
    if bytes.len() < 12 || &bytes[4..8] != b"ftyp" {
        return false;
    }
    // Marque principale, puis marques compatibles (dans la boîte ftyp, au début du fichier).
    let box_len = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    let end = box_len.clamp(12, bytes.len().min(256));
    let brands = &bytes[8..end];
    // 8..12 = marque principale ; 12..16 = version ; ensuite les marques compatibles.
    let major = &brands[..4.min(brands.len())];
    let compat = brands.get(8..).unwrap_or(&[]);
    major == b"avif" || major == b"avis" || compat.chunks_exact(4).any(|b| b == b"avif" || b == b"avis")
}

/// Décode un AVIF (AV1) en image 8 bits. Les profondeurs 10/12 bits sont ramenées à 8 bits.
/// Limite : la rotation/le miroir du conteneur (`irot`/`imir`) ne sont pas appliqués.
fn decode_avif(bytes: &[u8]) -> Result<DynamicImage, String> {
    use avif_decode::Image as A;
    let err = |what: &str| format!("AVIF : {what}");
    let image = avif_decode::Decoder::from_avif(bytes)
        .and_then(|d| d.to_image())
        .map_err(|e| err(&e.to_string()))?;

    let invalid = || err("tampon d'image invalide");
    Ok(match image {
        A::Rgb8(i) => {
            let (w, h) = (i.width() as u32, i.height() as u32);
            let mut v = Vec::with_capacity(w as usize * h as usize * 3);
            for p in i.pixels() {
                v.extend_from_slice(&[p.r, p.g, p.b]);
            }
            DynamicImage::ImageRgb8(RgbImage::from_raw(w, h, v).ok_or_else(invalid)?)
        }
        A::Rgb16(i) => {
            let (w, h) = (i.width() as u32, i.height() as u32);
            let mut v = Vec::with_capacity(w as usize * h as usize * 3);
            for p in i.pixels() {
                v.extend_from_slice(&[(p.r >> 8) as u8, (p.g >> 8) as u8, (p.b >> 8) as u8]);
            }
            DynamicImage::ImageRgb8(RgbImage::from_raw(w, h, v).ok_or_else(invalid)?)
        }
        A::Rgba8(i) => {
            let (w, h) = (i.width() as u32, i.height() as u32);
            let mut v = Vec::with_capacity(w as usize * h as usize * 4);
            for p in i.pixels() {
                v.extend_from_slice(&[p.r, p.g, p.b, p.a]);
            }
            DynamicImage::ImageRgba8(RgbaImage::from_raw(w, h, v).ok_or_else(invalid)?)
        }
        A::Rgba16(i) => {
            let (w, h) = (i.width() as u32, i.height() as u32);
            let mut v = Vec::with_capacity(w as usize * h as usize * 4);
            for p in i.pixels() {
                v.extend_from_slice(&[(p.r >> 8) as u8, (p.g >> 8) as u8, (p.b >> 8) as u8, (p.a >> 8) as u8]);
            }
            DynamicImage::ImageRgba8(RgbaImage::from_raw(w, h, v).ok_or_else(invalid)?)
        }
        A::Gray8(i) => {
            let (w, h) = (i.width() as u32, i.height() as u32);
            let mut v = Vec::with_capacity(w as usize * h as usize * 3);
            for p in i.pixels() {
                let g = p.value();
                v.extend_from_slice(&[g, g, g]);
            }
            DynamicImage::ImageRgb8(RgbImage::from_raw(w, h, v).ok_or_else(invalid)?)
        }
        A::Gray16(i) => {
            let (w, h) = (i.width() as u32, i.height() as u32);
            let mut v = Vec::with_capacity(w as usize * h as usize * 3);
            for p in i.pixels() {
                let g = (p.value() >> 8) as u8;
                v.extend_from_slice(&[g, g, g]);
            }
            DynamicImage::ImageRgb8(RgbImage::from_raw(w, h, v).ok_or_else(invalid)?)
        }
    })
}

fn resize(buf: Vec<u8>, sw: u32, sh: u32, dw: u32, dh: u32, pt: PixelType) -> Result<Vec<u8>, String> {
    let src = FirImage::from_vec_u8(sw, sh, buf, pt).map_err(|e| e.to_string())?;
    let mut dst = FirImage::new(dw, dh, pt);
    // Bilinéaire : très rapide et suffisamment net pour du tri visuel.
    let opts = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Bilinear));
    Resizer::new().resize(&src, &mut dst, &opts).map_err(|e| e.to_string())?;
    Ok(dst.into_vec())
}

/// Applique l'orientation EXIF à un tampon RGB(A) 8 bits. Retourne `(tampon, largeur, hauteur)`.
fn orient(buf: Vec<u8>, w: u32, h: u32, has_alpha: bool, o: Orientation) -> Result<(Vec<u8>, u32, u32), String> {
    if o == Orientation::NoTransforms {
        return Ok((buf, w, h));
    }
    let err = || "tampon d'image invalide".to_string();
    let mut dynimg = if has_alpha {
        DynamicImage::ImageRgba8(RgbaImage::from_raw(w, h, buf).ok_or_else(err)?)
    } else {
        DynamicImage::ImageRgb8(RgbImage::from_raw(w, h, buf).ok_or_else(err)?)
    };
    dynimg.apply_orientation(o);
    let (nw, nh) = (dynimg.width(), dynimg.height());
    let raw = if has_alpha { dynimg.into_rgba8().into_raw() } else { dynimg.into_rgb8().into_raw() };
    Ok((raw, nw, nh))
}

fn rgb_to_rgba(rgb: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgb.len() / 3 * 4);
    for px in rgb.chunks_exact(3) {
        out.extend_from_slice(&[px[0], px[1], px[2], 255]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, Rgba};

    fn rgb_image(w: u32, h: u32) -> RgbImage {
        // Dégradé horizontal rouge, vertical vert, bleu fixe : chaque coin est distinct.
        RgbImage::from_fn(w, h, |x, y| Rgb([(x * 255 / (w - 1).max(1)) as u8, (y * 255 / (h - 1).max(1)) as u8, 128]))
    }

    fn px(d: &Decoded, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * d.width + x) * 4) as usize;
        [d.rgba[i], d.rgba[i + 1], d.rgba[i + 2], d.rgba[i + 3]]
    }

    #[test]
    fn fit_size_never_upscales_and_keeps_ratio() {
        assert_eq!(fit_size(100, 50, 1000, 1000), (100, 50));
        assert_eq!(fit_size(4000, 3000, 1920, 1080), (1440, 1080));
        assert_eq!(fit_size(3000, 4000, 1920, 1080), (810, 1080));
        assert_eq!(fit_size(10000, 1, 100, 100), (100, 1));
        assert_eq!(fit_size(0, 0, 10, 10), (0, 0));
    }

    #[test]
    fn decodes_all_common_formats() {
        let tmp = tempfile::tempdir().unwrap();
        let img = rgb_image(64, 48);
        for ext in ["jpg", "jpeg", "png", "webp", "gif", "bmp", "tif", "tiff", "tga", "ico"] {
            let path = tmp.path().join(format!("img.{ext}"));
            let src = if ext == "ico" { DynamicImage::ImageRgba8(DynamicImage::ImageRgb8(rgb_image(32, 32)).to_rgba8()) } else { DynamicImage::ImageRgb8(img.clone()) };
            src.save(&path).unwrap_or_else(|e| panic!("écriture {ext} : {e}"));
            let d = decode_fit(&path, 4096, 4096).unwrap_or_else(|e| panic!("lecture {ext} : {e}"));
            let expect = if ext == "ico" { (32, 32) } else { (64, 48) };
            assert_eq!((d.width, d.height), expect, "{ext}");
            assert_eq!(d.rgba.len(), (d.width * d.height * 4) as usize, "{ext}");
        }
    }

    #[test]
    fn lossless_formats_keep_exact_pixels() {
        let tmp = tempfile::tempdir().unwrap();
        let img = rgb_image(16, 16);
        for ext in ["png", "bmp", "webp"] {
            let path = tmp.path().join(format!("x.{ext}"));
            img.save(&path).unwrap();
            let d = decode_fit(&path, 100, 100).unwrap();
            let p = px(&d, 15, 0);
            assert_eq!(p, [255, 0, 128, 255], "{ext}");
            assert_eq!(px(&d, 0, 15), [0, 255, 128, 255], "{ext}");
        }
    }

    #[test]
    fn large_images_are_downscaled_to_fit() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("big.png");
        rgb_image(400, 300).save(&path).unwrap();
        let d = decode_fit(&path, 100, 100).unwrap();
        assert_eq!((d.width, d.height), (100, 75));
        assert_eq!((d.orig_width, d.orig_height), (400, 300));
        assert_eq!(d.rgba.len(), 100 * 75 * 4);
        // Les couleurs sont préservées (coin haut-droit ~ rouge pur).
        let p = px(&d, 99, 0);
        assert!(p[0] > 240 && p[1] < 15 && (p[2] as i32 - 128).abs() < 4, "{p:?}");
    }

    #[test]
    fn alpha_is_preserved_and_not_premultiplied() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("a.png");
        RgbaImage::from_pixel(8, 8, Rgba([200, 100, 50, 128])).save(&path).unwrap();
        let d = decode_fit(&path, 100, 100).unwrap();
        assert_eq!(px(&d, 3, 3), [200, 100, 50, 128]);
        // Même après réduction.
        let d = decode_fit(&path, 4, 4).unwrap();
        let p = px(&d, 1, 1);
        assert!((p[0] as i32 - 200).abs() <= 2 && (p[3] as i32 - 128).abs() <= 2, "{p:?}");
    }

    #[test]
    fn format_is_guessed_from_content_not_extension() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("really_a_png.jpg");
        rgb_image(10, 10).save_with_format(&path, image::ImageFormat::Png).unwrap();
        let d = decode_fit(&path, 100, 100).unwrap();
        assert_eq!((d.width, d.height), (10, 10));
    }

    #[test]
    fn corrupt_and_missing_files_give_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let bad = tmp.path().join("bad.jpg");
        fs::write(&bad, b"ceci n'est pas une image").unwrap();
        assert!(decode_fit(&bad, 100, 100).is_err());

        let truncated = tmp.path().join("trunc.png");
        rgb_image(50, 50).save(&truncated).unwrap();
        let bytes = fs::read(&truncated).unwrap();
        fs::write(&truncated, &bytes[..bytes.len() / 3]).unwrap();
        assert!(decode_fit(&truncated, 100, 100).is_err());

        assert!(decode_fit(&tmp.path().join("missing.png"), 100, 100).is_err());
    }

    #[test]
    fn orientation_rotates_and_swaps_dimensions() {
        // 2x1 : pixel gauche rouge, pixel droit bleu.
        let buf = vec![255, 0, 0, 0, 0, 255];
        let (out, w, h) = orient(buf.clone(), 2, 1, false, Orientation::Rotate90).unwrap();
        assert_eq!((w, h), (1, 2));
        // Rotation de 90° horaire : le pixel gauche (rouge) passe en haut.
        assert_eq!(&out[..3], &[255, 0, 0]);
        assert_eq!(&out[3..], &[0, 0, 255]);

        let (same, w, h) = orient(buf.clone(), 2, 1, false, Orientation::NoTransforms).unwrap();
        assert_eq!((w, h, same), (2, 1, buf.clone()));

        let (flipped, ..) = orient(buf, 2, 1, false, Orientation::FlipHorizontal).unwrap();
        assert_eq!(flipped, vec![0, 0, 255, 255, 0, 0]);
    }

    // Fixtures générées une fois pour toutes avec `ravif` (voir tests/data/README.md).
    const GRADIENT_AVIF: &[u8] = include_bytes!("../tests/data/gradient.avif");
    const ALPHA_AVIF: &[u8] = include_bytes!("../tests/data/alpha.avif");

    fn close(a: u8, b: u8, tol: i32) -> bool {
        (a as i32 - b as i32).abs() <= tol
    }

    #[test]
    fn application_icon_is_a_valid_multi_size_ico() {
        let ico = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets").join("icon.ico");
        let d = decode_fit(&ico, 4096, 4096).unwrap(); // le décodeur ICO retient la plus grande taille
        assert_eq!((d.width, d.height), (256, 256));
        let entries = u16::from_le_bytes([fs::read(&ico).unwrap()[4], 0]);
        assert_eq!(entries, 7);
    }

    #[test]
    fn avif_signature_detection() {
        assert!(is_avif(GRADIENT_AVIF));
        assert!(is_avif(ALPHA_AVIF));
        assert!(!is_avif(b""));
        assert!(!is_avif(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"));
        assert!(!is_avif(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 16, b'J', b'F', b'I', b'F', 0, 1]));
        // HEIC (autre marque ftyp) : pas de l'AVIF.
        assert!(!is_avif(b"\0\0\0\x18ftypheic\0\0\0\0mif1heic"));
        // Marque principale quelconque mais « avif » parmi les marques compatibles.
        assert!(is_avif(b"\0\0\0\x18ftypmif1\0\0\0\0mif1avif"));
    }

    #[test]
    fn decodes_avif_with_correct_colors() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("gradient.avif");
        fs::write(&path, GRADIENT_AVIF).unwrap();
        let d = decode_fit(&path, 4096, 4096).unwrap();
        assert_eq!((d.width, d.height), (64, 48));
        let (tl, tr, bl) = (px(&d, 0, 0), px(&d, 63, 0), px(&d, 0, 47));
        assert!(close(tl[0], 0, 12) && close(tl[1], 0, 12) && close(tl[2], 128, 12), "{tl:?}");
        assert!(close(tr[0], 255, 12) && close(tr[1], 0, 12), "{tr:?}");
        assert!(close(bl[0], 0, 12) && close(bl[1], 255, 12), "{bl:?}");
        assert_eq!(tl[3], 255);
    }

    #[test]
    fn decodes_avif_alpha() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("alpha.avif");
        fs::write(&path, ALPHA_AVIF).unwrap();
        let d = decode_fit(&path, 4096, 4096).unwrap();
        assert_eq!((d.width, d.height), (32, 32));
        let p = px(&d, 10, 10);
        assert!(close(p[0], 200, 8) && close(p[1], 100, 8) && close(p[2], 50, 8) && close(p[3], 128, 4), "{p:?}");
    }

    #[test]
    fn avif_is_downscaled_and_detected_by_content() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("disguised.jpg"); // mauvaise extension
        fs::write(&path, GRADIENT_AVIF).unwrap();
        let d = decode_fit(&path, 32, 32).unwrap();
        assert_eq!((d.width, d.height), (32, 24));
        assert_eq!((d.orig_width, d.orig_height), (64, 48));
    }

    #[test]
    fn truncated_avif_is_an_error_not_a_crash() {
        let tmp = tempfile::tempdir().unwrap();
        for cut in [GRADIENT_AVIF.len() / 2, 40, 12] {
            let path = tmp.path().join(format!("cut{cut}.avif"));
            fs::write(&path, &GRADIENT_AVIF[..cut]).unwrap();
            assert!(decode_fit(&path, 100, 100).is_err(), "coupé à {cut} octets");
        }
    }

    /// Mesure indicative : `cargo test --release -- --ignored --nocapture bench_decode`
    #[test]
    #[ignore]
    fn bench_decode_12mp_jpeg() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("photo.jpg");
        // Dégradé + bruit déterministe, pour ressembler à une vraie photo.
        let mut seed = 12345u32;
        let img = RgbImage::from_fn(4000, 3000, |x, y| {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let n = (seed >> 24) as u8 / 8;
            Rgb([(x / 16) as u8 ^ n, (y / 12) as u8 ^ n, ((x + y) / 28) as u8 ^ n])
        });
        img.save(&path).unwrap();
        let size = fs::metadata(&path).unwrap().len();
        let _ = decode_fit(&path, 1920, 1080).unwrap(); // échauffement
        let start = std::time::Instant::now();
        let runs = 5;
        for _ in 0..runs {
            decode_fit(&path, 1920, 1080).unwrap();
        }
        println!("JPEG 4000x3000 ({} Mo) -> 1920x1080 : {:?} par image", size / 1_000_000, start.elapsed() / runs);
    }

    /// `IMAGESORT_BENCH_AVIF=chemin.avif cargo test --release --lib -- --ignored --nocapture bench_decode_avif`
    #[test]
    #[ignore]
    fn bench_decode_avif() {
        let Ok(path) = std::env::var("IMAGESORT_BENCH_AVIF") else {
            println!("IMAGESORT_BENCH_AVIF non défini : rien à mesurer");
            return;
        };
        let path = std::path::PathBuf::from(path);
        let _ = decode_fit(&path, 1920, 1080).unwrap();
        let start = std::time::Instant::now();
        let d = decode_fit(&path, 1920, 1080).unwrap();
        println!("AVIF {}x{} -> {}x{} : {:?}", d.orig_width, d.orig_height, d.width, d.height, start.elapsed());
    }

    #[test]
    fn rotated_orientation_swaps_axes_flag() {
        assert!(swaps_axes(Orientation::Rotate90));
        assert!(swaps_axes(Orientation::Rotate270FlipH));
        assert!(!swaps_axes(Orientation::Rotate180));
        assert!(!swaps_axes(Orientation::NoTransforms));
    }
}
