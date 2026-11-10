# ImageSort

Classe des images en les **déplaçant dans des sous-répertoires** (pas de tags) — pratique pour
préparer un jeu de données d'entraînement. Écrit en Rust, plein écran, avec préchargement.

## Utilisation

```text
ImageSort.exe "C:\chemin\TOSORT"
```

- Sans argument, un sélecteur de dossier s'ouvre (on peut aussi glisser le dossier sur l'exe).
- Les images à trier sont celles qui se trouvent **directement** dans `TOSORT`
  (les images déjà rangées dans les sous-répertoires ne sont pas affichées).
- Chaque sous-répertoire de `TOSORT`, **à tous les niveaux**, devient un bouton :
  `TOSORT\Alice`, `TOSORT\Alice\Bob`, `TOSORT\Eve` donnent les boutons `Alice`, `Alice/Bob`, `Eve`.
  Les répertoires cachés (`.git`…) et système (`$RECYCLE.BIN`…) sont ignorés.

## Touches

| Touche | Action |
|---|---|
| `←` / `→` (ou PgUp / PgDn) | image précédente / suivante (la touche maintenue défile) |
| `Home` / `End` | première / dernière image |
| `A`, `B`, `C`… | déplace l'image dans le répertoire du bouton correspondant (ordre alphabétique) puis affiche la suivante |
| `F1`…`F10` | répertoires n° 26 à 35 (les lettres `A`–`Y` couvrent les 25 premiers ; `Z` est réservée) |
| `Z` | annule la dernière action ; plusieurs appuis remontent dans l'historique |
| `Suppr` | met l'image à la corbeille Windows (annulable avec `Z`) |
| `Tab` | masque / affiche la barre de boutons |
| `F11` | plein écran / fenêtre |
| `Échap` | quitter |

Les boutons sont aussi cliquables à la souris. Maintenir une lettre ou `Suppr` n'agit qu'une fois
(pas de répétition automatique sur les actions qui modifient le disque).

Si un fichier du même nom existe déjà dans le répertoire cible, l'image est renommée
(`photo_1.jpg`) — rien n'est jamais écrasé.

## Formats

JPEG (jpg, jpeg, jpe, jfif), PNG, WebP, **AVIF**, GIF (1ʳᵉ image), BMP, TIFF, ICO, TGA, PNM.
L'orientation EXIF est respectée ; le format est reconnu d'après le contenu du fichier.

AVIF : décodage 100 % Rust (`rav1d` via `avif-decode`, copie locale dans `vendor/avif-decode`
dont seul `Cargo.toml` est modifié pour ne pas exiger NASM à la compilation). Les images 10/12 bits
sont ramenées à 8 bits ; la rotation/miroir du conteneur AVIF (`irot`/`imir`) n'est pas appliquée.
Mesure : AVIF 3000×2000 → 1620×1080 en ≈ 100 ms.

## Icône

`assets/icon.ico` (embarquée dans l'exe par `build.rs`) et `assets/icon.png` (icône de la fenêtre)
sont générées par `tools/make_icon.ps1` : une photo qui glisse dans un dossier, avec une touche « A ».

## Performances

- Les ~8 images suivantes et 2 précédentes sont décodées en arrière-plan (plusieurs threads),
  **réduites à la taille de l'écran** et envoyées au GPU avant d'être affichées.
  Passer à l'image suivante n'affiche donc qu'une texture déjà prête.
- Le cache est borné : ce qui sort de la fenêtre de préchargement est libéré.

## Développement

```text
cargo test                       # tests unitaires + test d'intégration pilotant l'UI sans écran
cargo build --release            # exécutable : target\release\ImageSort.exe
cargo test --release --lib -- --ignored --nocapture bench_decode   # mesure de vitesse de décodage
```

Organisation : `session.rs` (déplacement / suppression / annulation), `prefetch.rs` (préchargement),
`decode.rs` (décodage + réduction), `scan.rs`, `hotkeys.rs`, `natural.rs`, `view.rs`, `app.rs` (egui),
`tests/ui_flow.rs` (scénarios clavier de bout en bout).
