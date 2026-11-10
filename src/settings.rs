//! Réglages globaux (taille et position de la fenêtre), mémorisés entre deux lancements.
//!
//! Fichier texte `clé=valeur` dans `%APPDATA%\ImageSort\settings.txt`. Rien n'est jamais
//! bloquant : un fichier absent, illisible ou invalide donne simplement les valeurs par défaut.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Taille minimale acceptée pour une fenêtre mémorisée (points).
const MIN_SIZE: f32 = 200.0;
const MAX_SIZE: f32 = 20_000.0;
/// Tolérance (points) en dessous de laquelle un changement de taille/position est ignoré.
const EPSILON: f32 = 0.5;

#[derive(Debug, Clone, PartialEq)]
pub struct WindowSettings {
    /// Plein écran (par défaut : oui, comme avant).
    pub fullscreen: bool,
    /// Fenêtre agrandie (maximisée).
    pub maximized: bool,
    /// Taille intérieure de la fenêtre en mode fenêtré, en points.
    pub size: Option<(f32, f32)>,
    /// Position extérieure de la fenêtre en mode fenêtré, en points.
    pub pos: Option<(f32, f32)>,
    /// Échelle (pixels par point) lors de l'enregistrement : sert à vérifier que la position est visible.
    pub ppp: f32,
}

impl Default for WindowSettings {
    fn default() -> Self {
        WindowSettings { fullscreen: true, maximized: false, size: None, pos: None, ppp: 1.0 }
    }
}

/// Ce que l'on sait de la fenêtre à un instant donné (tout est optionnel : certaines
/// informations n'arrivent qu'après la première image).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ViewState {
    pub fullscreen: Option<bool>,
    pub maximized: Option<bool>,
    pub minimized: Option<bool>,
    /// Taille intérieure.
    pub inner: Option<(f32, f32)>,
    /// Coin supérieur gauche de la fenêtre (bordures comprises).
    pub outer_min: Option<(f32, f32)>,
    /// Taille de l'écran qui contient la fenêtre.
    pub monitor: Option<(f32, f32)>,
    pub ppp: Option<f32>,
}

impl WindowSettings {
    pub fn parse(text: &str) -> Self {
        let mut s = WindowSettings::default();
        let (mut w, mut h, mut x, mut y) = (None, None, None, None);
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            let (key, value) = (key.trim(), value.trim());
            let num = || value.parse::<f32>().ok().filter(|v| v.is_finite());
            match key {
                "fullscreen" => s.fullscreen = value == "true",
                "maximized" => s.maximized = value == "true",
                "width" => w = num(),
                "height" => h = num(),
                "x" => x = num(),
                "y" => y = num(),
                "ppp" => s.ppp = num().filter(|p| (0.25..=8.0).contains(p)).unwrap_or(1.0),
                _ => {}
            }
        }
        s.size = match (w, h) {
            (Some(w), Some(h)) if (MIN_SIZE..=MAX_SIZE).contains(&w) && (MIN_SIZE..=MAX_SIZE).contains(&h) => Some((w, h)),
            _ => None,
        };
        s.pos = x.zip(y);
        s
    }

    pub fn to_text(&self) -> String {
        let mut t = format!("fullscreen={}\nmaximized={}\nppp={}\n", self.fullscreen, self.maximized, self.ppp);
        if let Some((w, h)) = self.size {
            t.push_str(&format!("width={w}\nheight={h}\n"));
        }
        if let Some((x, y)) = self.pos {
            t.push_str(&format!("x={x}\ny={y}\n"));
        }
        t
    }

    /// Lit les réglages ; valeurs par défaut si le fichier est absent ou illisible.
    pub fn load(path: &Path) -> Self {
        fs::read_to_string(path).map(|t| Self::parse(&t)).unwrap_or_default()
    }

    /// Écrit les réglages (via un fichier temporaire, pour ne jamais laisser un fichier tronqué).
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, self.to_text())?;
        fs::rename(&tmp, path).or_else(|_| {
            let r = fs::write(path, self.to_text());
            let _ = fs::remove_file(&tmp);
            r
        })
    }

    /// Emplacement par défaut du fichier de réglages.
    pub fn default_path() -> Option<PathBuf> {
        let base = std::env::var_os("APPDATA").or_else(|| std::env::var_os("XDG_CONFIG_HOME"))?;
        Some(PathBuf::from(base).join("ImageSort").join("settings.txt"))
    }

    /// Position à utiliser au lancement : la position mémorisée, seulement si la fenêtre
    /// serait visible sur l'un des écrans actuels (écran débranché, résolution changée…).
    pub fn startup_position(&self) -> Option<(f32, f32)> {
        let pos = self.pos?;
        match virtual_screen() {
            Some(screen) if !position_is_visible(pos, self.ppp, screen) => None,
            _ => Some(pos),
        }
    }

    /// Met à jour les réglages d'après l'état courant de la fenêtre. `true` si quelque chose a changé.
    pub fn observe(&mut self, v: &ViewState) -> bool {
        // Une fenêtre réduite annonce des coordonnées absurdes (−32000) : on n'en retient rien.
        if v.minimized == Some(true) {
            return false;
        }
        let before = self.clone();
        if let Some(f) = v.fullscreen {
            self.fullscreen = f;
        }
        if !self.fullscreen {
            if let Some(m) = v.maximized {
                self.maximized = m;
            }
        }
        // La taille/position « normales » ne se retiennent qu'en fenêtre ordinaire :
        // ni plein écran, ni agrandie (on garde alors celles d'avant).
        if !self.fullscreen && !self.maximized && v.fullscreen != Some(true) {
            if let Some((w, h)) = v.inner {
                let fills_monitor = v.monitor.is_some_and(|(mw, mh)| w >= mw - 1.0 && h >= mh - 1.0);
                if (MIN_SIZE..=MAX_SIZE).contains(&w) && (MIN_SIZE..=MAX_SIZE).contains(&h) && !fills_monitor {
                    self.size = Some((w, h));
                    if let Some(p) = v.outer_min {
                        self.pos = Some(p);
                    }
                    if let Some(ppp) = v.ppp.filter(|p| *p > 0.0) {
                        self.ppp = ppp;
                    }
                }
            }
        }
        !self.same_as(&before)
    }

    fn same_as(&self, o: &WindowSettings) -> bool {
        let close = |a: Option<(f32, f32)>, b: Option<(f32, f32)>| match (a, b) {
            (None, None) => true,
            (Some(a), Some(b)) => (a.0 - b.0).abs() < EPSILON && (a.1 - b.1).abs() < EPSILON,
            _ => false,
        };
        self.fullscreen == o.fullscreen
            && self.maximized == o.maximized
            && (self.ppp - o.ppp).abs() < 0.01
            && close(self.size, o.size)
            && close(self.pos, o.pos)
    }
}

/// Le coin supérieur gauche (barre de titre) de la fenêtre est-il dans la zone d'affichage
/// `screen` = (x, y, largeur, hauteur) en pixels ?
pub fn position_is_visible(pos: (f32, f32), ppp: f32, screen: (i32, i32, i32, i32)) -> bool {
    // Un point de la barre de titre, un peu à l'intérieur de la fenêtre.
    let px = (pos.0 + 40.0) * ppp;
    let py = (pos.1 + 10.0) * ppp;
    let (sx, sy, sw, sh) = (screen.0 as f32, screen.1 as f32, screen.2 as f32, screen.3 as f32);
    px >= sx && px < sx + sw && py >= sy && py < sy + sh
}

/// Zone couverte par l'ensemble des écrans (x, y, largeur, hauteur), en pixels.
#[cfg(windows)]
fn virtual_screen() -> Option<(i32, i32, i32, i32)> {
    #[link(name = "user32")]
    extern "system" {
        fn GetSystemMetrics(index: i32) -> i32;
    }
    // SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN
    let (x, y, w, h) = unsafe { (GetSystemMetrics(76), GetSystemMetrics(77), GetSystemMetrics(78), GetSystemMetrics(79)) };
    (w > 0 && h > 0).then_some((x, y, w, h))
}

#[cfg(not(windows))]
fn virtual_screen() -> Option<(i32, i32, i32, i32)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn windowed(w: f32, h: f32, x: f32, y: f32) -> ViewState {
        ViewState {
            fullscreen: Some(false),
            maximized: Some(false),
            minimized: Some(false),
            inner: Some((w, h)),
            outer_min: Some((x, y)),
            monitor: Some((1920.0, 1080.0)),
            ppp: Some(1.0),
        }
    }

    #[test]
    fn default_is_fullscreen_like_before() {
        let s = WindowSettings::default();
        assert!(s.fullscreen && !s.maximized && s.size.is_none() && s.pos.is_none());
    }

    #[test]
    fn text_roundtrip() {
        let s = WindowSettings { fullscreen: false, maximized: true, size: Some((1280.5, 720.0)), pos: Some((-30.0, 42.0)), ppp: 1.25 };
        assert_eq!(WindowSettings::parse(&s.to_text()), s);
        let d = WindowSettings::default();
        assert_eq!(WindowSettings::parse(&d.to_text()), d);
    }

    #[test]
    fn garbage_gives_defaults() {
        assert_eq!(WindowSettings::parse(""), WindowSettings::default());
        assert_eq!(WindowSettings::parse("n'importe quoi\n===\n\u{0}"), WindowSettings::default());
        // Taille absurde ou incomplète : ignorée ; le reste est conservé.
        let s = WindowSettings::parse("fullscreen=false\nwidth=3\nheight=900\nx=10\ny=abc\nppp=NaN\n");
        assert!(!s.fullscreen);
        assert_eq!(s.size, None);
        assert_eq!(s.pos, None);
        assert_eq!(s.ppp, 1.0);
        let s = WindowSettings::parse("width=inf\nheight=100000\n");
        assert_eq!(s.size, None);
    }

    #[test]
    fn save_and_load_through_a_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("sub").join("settings.txt");
        assert_eq!(WindowSettings::load(&path), WindowSettings::default());
        let s = WindowSettings { fullscreen: false, maximized: false, size: Some((900.0, 600.0)), pos: Some((100.0, 80.0)), ppp: 1.5 };
        s.save(&path).unwrap();
        assert_eq!(WindowSettings::load(&path), s);
        // Écrasement.
        let t = WindowSettings { fullscreen: true, ..s.clone() };
        t.save(&path).unwrap();
        assert_eq!(WindowSettings::load(&path), t);
        assert!(!path.with_extension("tmp").exists());
    }

    #[test]
    fn observe_records_a_normal_window() {
        let mut s = WindowSettings::default();
        assert!(s.observe(&windowed(1000.0, 700.0, 50.0, 60.0)));
        assert!(!s.fullscreen);
        assert_eq!(s.size, Some((1000.0, 700.0)));
        assert_eq!(s.pos, Some((50.0, 60.0)));
        // Même état : rien ne change (pas d'écriture inutile).
        assert!(!s.observe(&windowed(1000.2, 700.0, 50.0, 60.0)));
        // Redimensionnement.
        assert!(s.observe(&windowed(1100.0, 700.0, 50.0, 60.0)));
        assert_eq!(s.size, Some((1100.0, 700.0)));
    }

    #[test]
    fn observe_keeps_the_windowed_size_when_going_fullscreen() {
        let mut s = WindowSettings::default();
        s.observe(&windowed(1000.0, 700.0, 50.0, 60.0));
        // Passage en plein écran : le drapeau change, mais la taille d'écran n'est pas retenue.
        let fs = ViewState { fullscreen: Some(true), inner: Some((1920.0, 1080.0)), outer_min: Some((0.0, 0.0)), monitor: Some((1920.0, 1080.0)), ..windowed(0.0, 0.0, 0.0, 0.0) };
        assert!(s.observe(&fs));
        assert!(s.fullscreen);
        assert_eq!(s.size, Some((1000.0, 700.0)));
        assert_eq!(s.pos, Some((50.0, 60.0)));
        // Retour en fenêtre.
        assert!(s.observe(&windowed(1000.0, 700.0, 50.0, 60.0)));
        assert!(!s.fullscreen);
    }

    #[test]
    fn observe_ignores_the_frame_where_fullscreen_flag_lags() {
        // La taille est déjà celle de l'écran alors que le drapeau n'est pas encore à jour.
        let mut s = WindowSettings::default();
        s.observe(&windowed(1000.0, 700.0, 50.0, 60.0));
        s.observe(&windowed(1920.0, 1080.0, 0.0, 0.0));
        assert_eq!(s.size, Some((1000.0, 700.0)));
    }

    #[test]
    fn observe_ignores_minimized_and_missing_info() {
        let mut s = WindowSettings::default();
        s.observe(&windowed(1000.0, 700.0, 50.0, 60.0));
        let before = s.clone();
        let min = ViewState { minimized: Some(true), outer_min: Some((-32000.0, -32000.0)), inner: Some((160.0, 28.0)), ..windowed(0.0, 0.0, 0.0, 0.0) };
        assert!(!s.observe(&min));
        assert!(!s.observe(&ViewState::default()));
        assert_eq!(s, before);
    }

    #[test]
    fn observe_maximized_keeps_the_restore_size() {
        let mut s = WindowSettings::default();
        s.observe(&windowed(1000.0, 700.0, 50.0, 60.0));
        let max = ViewState { maximized: Some(true), ..windowed(1900.0, 1000.0, -8.0, -8.0) };
        assert!(s.observe(&max));
        assert!(s.maximized && !s.fullscreen);
        assert_eq!(s.size, Some((1000.0, 700.0)));
        assert_eq!(s.pos, Some((50.0, 60.0)));
    }

    #[test]
    fn position_visibility() {
        let screen = (0, 0, 1920, 1080);
        assert!(position_is_visible((100.0, 100.0), 1.0, screen));
        assert!(position_is_visible((-30.0, 0.0), 1.0, screen)); // un peu à gauche : barre de titre visible
        assert!(!position_is_visible((-32000.0, -32000.0), 1.0, screen));
        assert!(!position_is_visible((2500.0, 100.0), 1.0, screen)); // écran de droite débranché
        assert!(!position_is_visible((100.0, 1200.0), 1.0, screen));
        // Avec une mise à l'échelle de 150 %, les points sont convertis en pixels.
        assert!(!position_is_visible((1400.0, 100.0), 1.5, screen));
        assert!(position_is_visible((1000.0, 100.0), 1.5, screen));
        // Second écran à gauche (coordonnées négatives).
        assert!(position_is_visible((-1500.0, 100.0), 1.0, (-1920, 0, 3840, 1080)));
    }
}
