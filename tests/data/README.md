# Fixtures de test

- `gradient.avif` : dégradé 64×48 opaque (R = x, G = y, B = 128), AVIF 4:4:4 RGB, qualité 100.
- `alpha.avif` : 32×32 uni, RGBA (200, 100, 50, 128).

Générés avec la crate `ravif` 0.13 (`Encoder::new().with_quality(100.0).with_speed(8)
.with_internal_color_model(ColorModel::RGB)`), puis figés ici pour que les tests n'aient pas
besoin d'un encodeur AVIF.
