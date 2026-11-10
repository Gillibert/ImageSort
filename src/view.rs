//! Géométrie d'affichage.

/// Rectangle `(x, y, largeur, hauteur)`.
pub type Rect4 = (f32, f32, f32, f32);

/// Plus grand rectangle de ratio `image` contenu (et centré) dans `avail`
/// (agrandit ou réduit). Retourne `(x, y, w, h)` relatif au coin haut-gauche de `avail`.
pub fn fit_rect(avail: (f32, f32), image: (f32, f32)) -> Rect4 {
    let (aw, ah) = avail;
    let (iw, ih) = image;
    if aw <= 0.0 || ah <= 0.0 || iw <= 0.0 || ih <= 0.0 {
        return (0.0, 0.0, 0.0, 0.0);
    }
    let scale = (aw / iw).min(ah / ih);
    let (w, h) = (iw * scale, ih * scale);
    ((aw - w) / 2.0, (ah - h) / 2.0, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_image_is_letterboxed_vertically() {
        let (x, y, w, h) = fit_rect((1000.0, 1000.0), (200.0, 100.0));
        assert_eq!((x, w, h), (0.0, 1000.0, 500.0));
        assert_eq!(y, 250.0);
    }

    #[test]
    fn tall_image_is_pillarboxed() {
        let (x, y, w, h) = fit_rect((1920.0, 1080.0), (1000.0, 2000.0));
        assert_eq!((y, h), (0.0, 1080.0));
        assert_eq!(w, 540.0);
        assert_eq!(x, (1920.0 - 540.0) / 2.0);
    }

    #[test]
    fn small_images_are_upscaled() {
        let (_, _, w, h) = fit_rect((1920.0, 1080.0), (64.0, 64.0));
        assert_eq!((w, h), (1080.0, 1080.0));
    }

    #[test]
    fn degenerate_inputs() {
        assert_eq!(fit_rect((0.0, 10.0), (5.0, 5.0)), (0.0, 0.0, 0.0, 0.0));
        assert_eq!(fit_rect((10.0, 10.0), (0.0, 5.0)), (0.0, 0.0, 0.0, 0.0));
    }
}
