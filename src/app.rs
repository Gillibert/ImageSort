//! Interface plein écran (egui / eframe).

use crate::decode::decode_fit;
use crate::hotkeys::{self, Hotkey};
use crate::prefetch::Prefetcher;
use crate::session::{Session, SessionError, Undone};
use crate::settings::{ViewState, WindowSettings};
use crate::view::fit_rect;
use egui::{Align2, Color32, FontId, Key, Pos2, Rect, RichText, TextureHandle, TextureOptions, Vec2};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Image prête à l'affichage : la texture est déjà créée (par un thread de fond).
pub struct Loaded {
    texture: TextureHandle,
    /// Taille de la texture (après réduction).
    size: (u32, u32),
    /// Taille de l'image d'origine.
    orig: (u32, u32),
}

/// Textes affichés par l'interface. Les polices intégrées d'egui n'ont pas les flèches
/// (← → ↑ s'affichent en carré) : on n'en utilise pas, et un test vérifie que chaque
/// caractère de ces textes existe dans les polices.
pub mod texts {
    pub const HELP: &str = "Gauche / Droite : naviguer   ·   lettre : déplacer   ·   Haut : dossier parent   ·   Z : annuler   ·   Suppr : corbeille   ·   Tab : barre   ·   F11 : plein écran   ·   Échap : quitter";
    pub const LOADING: &str = "Chargement…";
    pub const NO_IMAGE: &str = "Aucune image. Créez des sous-répertoires dans le dossier à trier.";
    pub const ALL_SORTED: &str = "Tout est trié !  (Z pour annuler)";
    pub const NO_SUBFOLDER: &str = "Aucun sous-répertoire : créez-en dans le dossier à trier (Alice, Bob…).";

    pub fn moved(name: &str) -> String {
        format!("Déplacée dans {name}")
    }
    pub fn moved_up(name: &str) -> String {
        format!("Déplacée dans {name} (dossier parent)")
    }
    pub fn parent_button(name: &str) -> String {
        format!("[Haut]  {name} (dossier parent)")
    }
    pub fn dimensions(w: u32, h: u32) -> String {
        format!("   {w}×{h}")
    }
    pub fn info(file: &str, index: usize, len: usize, dims: &str, undo: usize) -> String {
        format!("{file}   {index} / {len}{dims}   (annuler : {undo})")
    }

    /// Tous les textes (avec des exemples pour les parties variables), pour le test de couverture des polices.
    pub fn all_samples() -> Vec<String> {
        vec![
            HELP.into(),
            LOADING.into(),
            NO_IMAGE.into(),
            ALL_SORTED.into(),
            NO_SUBFOLDER.into(),
            moved("Alice/Bob"),
            moved_up("TRUC"),
            parent_button("TRUC"),
            info("photo.jpg", 3, 120, &dimensions(4032, 3024), 2),
            "Supprimée (corbeille)".into(),
            "Rien à annuler".into(),
            "Annulé : photo.jpg revient de Alice".into(),
            "Annulé : photo.jpg restaurée".into(),
            "Image illisible : format non reconnu".into(),
            "Erreur : fichier disparu".into(),
        ]
    }
}

/// Actions déclenchées par le clavier ou la souris.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cmd {
    Next,
    Prev,
    First,
    Last,
    MoveTo(usize),
    /// Vers le dossier parent du dossier trié.
    MoveUp,
    Delete,
    Undo,
    ToggleBar,
    ToggleFullscreen,
    Quit,
}

struct Status {
    text: String,
    error: bool,
    since: Instant,
}

const STATUS_DURATION: Duration = Duration::from_millis(2500);

pub struct SorterApp {
    session: Session,
    prefetch: Prefetcher<Loaded>,
    /// Taille maximale (pixels) des textures : celle de l'écran.
    target_px: Arc<Mutex<(u32, u32)>>,
    hotkeys: Vec<Option<Hotkey>>,
    /// Touche clavier -> indice du répertoire cible.
    key_map: Vec<(Key, usize)>,
    forward: bool,
    show_bar: bool,
    status: Option<Status>,
    /// Mémorisation de la taille/position de la fenêtre (inactive tant qu'on n'a pas appelé `remember_window`).
    window: Option<WindowMemory>,
}

/// Délai avant d'écrire sur le disque un changement de taille/position de la fenêtre
/// (évite d'écrire à chaque image d'un redimensionnement à la souris).
const SAVE_DELAY: Duration = Duration::from_millis(500);

struct WindowMemory {
    path: PathBuf,
    settings: WindowSettings,
    /// Moment du premier changement non encore écrit.
    dirty_since: Option<Instant>,
}

impl SorterApp {
    pub fn new(egui_ctx: &egui::Context, session: Session) -> Self {
        let target_px = Arc::new(Mutex::new((1920u32, 1080u32)));
        let ctx = egui_ctx.clone();
        let tgt = Arc::clone(&target_px);
        let ctx_ready = egui_ctx.clone();

        let loader = move |path: &Path| -> Result<Loaded, String> {
            let (tw, th) = *tgt.lock().unwrap_or_else(|p| p.into_inner());
            let d = decode_fit(path, tw, th)?;
            let image = egui::ColorImage::from_rgba_unmultiplied([d.width as usize, d.height as usize], &d.rgba);
            // Petites images (vignettes d'entraînement…) : agrandissement net plutôt que flou.
            let upscale = (tw as f32 / d.orig_width as f32).min(th as f32 / d.orig_height as f32);
            let options = if upscale >= 3.0 {
                TextureOptions { magnification: egui::TextureFilter::Nearest, ..TextureOptions::LINEAR }
            } else {
                TextureOptions::LINEAR
            };
            let texture = ctx.load_texture(path.to_string_lossy(), image, options);
            Ok(Loaded { texture, size: (d.width, d.height), orig: (d.orig_width, d.orig_height) })
        };
        // Chaque image prête provoque un rafraîchissement (et donc l'envoi de sa texture au GPU).
        let on_ready = move || ctx_ready.request_repaint();

        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        let prefetch = Prefetcher::new((cores / 2).clamp(2, 6), loader, on_ready);

        let hotkeys = hotkeys::assign(session.targets().len());
        let key_map = hotkeys
            .iter()
            .enumerate()
            .filter_map(|(i, h)| h.and_then(|h| Key::from_name(&h.label())).map(|k| (k, i)))
            .collect();

        SorterApp { session, prefetch, target_px, hotkeys, key_map, forward: true, show_bar: true, status: None, window: None }
    }

    /// Active la mémorisation de la taille/position de la fenêtre dans `path`
    /// (`initial` : réglages déjà lus au lancement).
    pub fn remember_window(&mut self, path: PathBuf, initial: WindowSettings) {
        self.window = Some(WindowMemory { path, settings: initial, dirty_since: None });
    }

    /// Réglages de fenêtre actuellement retenus (pour les tests).
    pub fn window_settings(&self) -> Option<&WindowSettings> {
        self.window.as_ref().map(|w| &w.settings)
    }

    /// Écrit tout de suite les réglages de fenêtre (à la fermeture). Les erreurs sont ignorées :
    /// ne pas pouvoir mémoriser la taille ne doit jamais gêner le tri.
    pub fn save_window_now(&mut self) {
        if let Some(w) = &mut self.window {
            let _ = w.settings.save(&w.path);
            w.dirty_since = None;
        }
    }

    /// Suit la taille/position de la fenêtre ; écrit les changements après un court délai.
    fn track_window(&mut self, ctx: &egui::Context) {
        let Some(w) = &mut self.window else { return };
        let view = ctx.input(|i| {
            let vp = i.viewport();
            ViewState {
                fullscreen: vp.fullscreen,
                maximized: vp.maximized,
                minimized: vp.minimized,
                inner: vp.inner_rect.map(|r| (r.width(), r.height())),
                outer_min: vp.outer_rect.map(|r| (r.min.x, r.min.y)),
                monitor: vp.monitor_size.map(|s| (s.x, s.y)),
                ppp: vp.native_pixels_per_point,
            }
        });
        if w.settings.observe(&view) && w.dirty_since.is_none() {
            w.dirty_since = Some(Instant::now());
        }
        if let Some(since) = w.dirty_since {
            let age = since.elapsed();
            if age >= SAVE_DELAY {
                let _ = w.settings.save(&w.path);
                w.dirty_since = None;
            } else {
                ctx.request_repaint_after(SAVE_DELAY - age);
            }
        }
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    /// `true` si l'image courante est déjà décodée et prête à être affichée (ou s'il n'y en a plus).
    pub fn current_ready(&self) -> bool {
        self.session.current().map_or(true, |p| self.prefetch.get(p).is_some())
    }

    /// Nombre d'images actuellement préchargées en mémoire.
    pub fn cached_count(&self) -> usize {
        self.prefetch.cached()
    }

    /// Dernier message d'état affiché (pour les tests).
    pub fn status_text(&self) -> Option<&str> {
        self.status.as_ref().map(|s| s.text.as_str())
    }

    fn set_status(&mut self, text: impl Into<String>, error: bool) {
        self.status = Some(Status { text: text.into(), error, since: Instant::now() });
    }

    /// Adapte la taille des textures à l'écran (change au démarrage, puis rarement).
    fn sync_target_size(&mut self, ctx: &egui::Context) {
        let (monitor, ppp) = ctx.input(|i| (i.viewport().monitor_size, i.viewport().native_pixels_per_point.unwrap_or(i.pixels_per_point)));
        let size = monitor.unwrap_or_else(|| ctx.content_rect().size());
        let px = ((size.x * ppp).round() as u32, (size.y * ppp).round() as u32);
        if px.0 < 64 || px.1 < 64 {
            return;
        }
        let mut cur = self.target_px.lock().unwrap_or_else(|p| p.into_inner());
        if *cur != px {
            *cur = px;
            drop(cur);
            self.prefetch.clear();
        }
    }

    fn read_commands(&self, ctx: &egui::Context) -> Vec<Cmd> {
        let mut cmds = Vec::new();
        ctx.input(|i| {
            for ev in &i.events {
                let egui::Event::Key { key, pressed: true, repeat, modifiers, .. } = ev else { continue };
                if modifiers.ctrl || modifiers.alt || modifiers.command {
                    continue;
                }
                match key {
                    // Navigation et annulation : la répétition automatique est utile.
                    Key::ArrowRight | Key::PageDown => cmds.push(Cmd::Next),
                    Key::ArrowLeft | Key::PageUp => cmds.push(Cmd::Prev),
                    Key::Z => cmds.push(Cmd::Undo),
                    // Actions qui modifient le disque : jamais en répétition automatique.
                    _ if *repeat => {}
                    Key::ArrowUp => cmds.push(Cmd::MoveUp),
                    Key::Home => cmds.push(Cmd::First),
                    Key::End => cmds.push(Cmd::Last),
                    Key::Delete => cmds.push(Cmd::Delete),
                    Key::Tab => cmds.push(Cmd::ToggleBar),
                    Key::F11 => cmds.push(Cmd::ToggleFullscreen),
                    Key::Escape => cmds.push(Cmd::Quit),
                    other => {
                        if let Some((_, idx)) = self.key_map.iter().find(|(k, _)| k == other) {
                            cmds.push(Cmd::MoveTo(*idx));
                        }
                    }
                }
            }
        });
        cmds
    }

    fn apply(&mut self, ctx: &egui::Context, cmd: Cmd) {
        match cmd {
            Cmd::Next => {
                self.forward = true;
                self.session.next();
            }
            Cmd::Prev => {
                self.forward = false;
                self.session.prev();
            }
            Cmd::First => {
                self.forward = true;
                self.session.first();
            }
            Cmd::Last => {
                self.forward = false;
                self.session.last();
            }
            Cmd::MoveTo(i) => {
                let name = self.session.targets().get(i).map(|t| t.name.clone()).unwrap_or_default();
                match self.session.move_to(i) {
                    Ok(_) => {
                        self.forward = true;
                        self.set_status(texts::moved(&name), false)
                    }
                    Err(e) => self.report(e),
                }
            }
            Cmd::MoveUp => {
                let name = self.session.parent().map(|t| t.name.clone()).unwrap_or_default();
                match self.session.move_up() {
                    Ok(_) => {
                        self.forward = true;
                        self.set_status(texts::moved_up(&name), false)
                    }
                    Err(e) => self.report(e),
                }
            }
            Cmd::Delete => match self.session.delete() {
                Ok(_) => {
                    self.forward = true;
                    self.set_status("Supprimée (corbeille)", false)
                }
                Err(e) => self.report(e),
            },
            Cmd::Undo => match self.session.undo() {
                Ok(Some(Undone::Moved { file, from })) => {
                    let to = from.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    self.set_status(format!("Annulé : {} revient de {to}", file_name(&file)), false)
                }
                Ok(Some(Undone::Deleted { file })) => self.set_status(format!("Annulé : {} restaurée", file_name(&file)), false),
                Ok(None) => self.set_status("Rien à annuler", true),
                Err(e) => self.report(e),
            },
            Cmd::ToggleBar => self.show_bar = !self.show_bar,
            Cmd::ToggleFullscreen => {
                let fs = ctx.input(|i| i.viewport().fullscreen).unwrap_or(true);
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fs));
                // Sortie du plein écran : on retrouve la taille/position mémorisées (si l'appli a démarré
                // en plein écran, la fenêtre ne les connaît pas).
                if fs {
                    if let Some(w) = &self.window {
                        if let Some((sw, sh)) = w.settings.size {
                            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(sw, sh)));
                        }
                        if let Some((x, y)) = w.settings.startup_position() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(Pos2::new(x, y)));
                        }
                    }
                }
            }
            Cmd::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
        ctx.request_repaint();
    }

    fn report(&mut self, e: SessionError) {
        self.set_status(format!("Erreur : {e}"), true);
    }

    fn draw_status(&mut self, ctx: &egui::Context, painter: &egui::Painter, screen: Rect) {
        let Some(st) = &self.status else { return };
        let age = st.since.elapsed();
        if age > STATUS_DURATION {
            self.status = None;
            return;
        }
        ctx.request_repaint_after(STATUS_DURATION - age);
        let color = if st.error { Color32::from_rgb(255, 120, 110) } else { Color32::from_rgb(140, 230, 150) };
        let font = FontId::proportional(26.0);
        let galley = painter.layout_no_wrap(st.text.clone(), font, color);
        let pos = Pos2::new(screen.right() - galley.size().x - 24.0, screen.top() + 16.0);
        let bg = Rect::from_min_size(pos, galley.size()).expand2(Vec2::new(12.0, 6.0));
        painter.rect_filled(bg, 8, Color32::from_black_alpha(190));
        painter.galley(pos, galley, color);
    }
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

impl eframe::App for SorterApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.step(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.paint(ui);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 1.0]
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_window_now();
    }
}

impl SorterApp {
    /// Traite les entrées clavier et met à jour le préchargement (avant le dessin).
    pub fn step(&mut self, ctx: &egui::Context) {
        self.sync_target_size(ctx);
        self.track_window(ctx);
        for cmd in self.read_commands(ctx) {
            self.apply(ctx, cmd);
        }
        // Précharge autour de l'image courante (no-op si la liste n'a pas changé).
        self.prefetch.set_wanted(self.session.prefetch_window(self.forward));
    }

    /// Dessine l'image courante, les informations et la barre de boutons.
    pub fn paint(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let screen = ui.max_rect();
        let painter = ui.painter().clone();
        painter.rect_filled(screen, 0, Color32::BLACK);

        let centered = |text: &str, color: Color32| {
            painter.text(screen.center(), Align2::CENTER_CENTER, text, FontId::proportional(32.0), color);
        };

        match self.session.current() {
            None => {
                if self.session.targets().is_empty() {
                    centered(texts::NO_IMAGE, Color32::GRAY);
                } else {
                    centered(texts::ALL_SORTED, Color32::from_rgb(140, 230, 150));
                }
            }
            Some(path) => match self.prefetch.get(path) {
                Some(res) => match &*res {
                    Ok(img) => {
                        let (x, y, w, h) = fit_rect((screen.width(), screen.height()), (img.size.0 as f32, img.size.1 as f32));
                        let rect = Rect::from_min_size(screen.min + Vec2::new(x, y), Vec2::new(w, h));
                        painter.image(img.texture.id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                        self.draw_info(&painter, screen, path, Some(img.orig));
                    }
                    Err(e) => {
                        centered(&format!("Image illisible : {e}"), Color32::from_rgb(255, 120, 110));
                        self.draw_info(&painter, screen, path, None);
                    }
                },
                None => {
                    centered(texts::LOADING, Color32::GRAY);
                    self.draw_info(&painter, screen, path, None);
                }
            },
        }

        self.draw_status(&ctx, &painter, screen);

        if self.show_bar {
            if let Some(cmd) = self.draw_bar(&ctx, screen) {
                self.apply(&ctx, cmd);
            }
        }
    }

    fn draw_info(&self, painter: &egui::Painter, screen: Rect, path: &Path, orig: Option<(u32, u32)>) {
        let dims = orig.map(|(w, h)| texts::dimensions(w, h)).unwrap_or_default();
        let text = texts::info(&file_name(path), self.session.index() + 1, self.session.len(), &dims, self.session.undo_len());
        let galley = painter.layout_no_wrap(text, FontId::proportional(18.0), Color32::from_gray(225));
        let pos = screen.min + Vec2::new(16.0, 14.0);
        painter.rect_filled(Rect::from_min_size(pos, galley.size()).expand2(Vec2::new(8.0, 4.0)), 6, Color32::from_black_alpha(150));
        painter.galley(pos, galley, Color32::from_gray(225));
    }

    /// Barre de boutons en bas de l'écran. Retourne l'action demandée par un clic, le cas échéant.
    fn draw_bar(&self, ctx: &egui::Context, screen: Rect) -> Option<Cmd> {
        let mut clicked = None;
        egui::Area::new(egui::Id::unique("target_bar"))
            .order(egui::Order::Foreground)
            .anchor(Align2::CENTER_BOTTOM, Vec2::new(0.0, -12.0))
            .show(ctx, |ui| {
                egui::Frame::new().fill(Color32::from_black_alpha(205)).corner_radius(10).inner_margin(10).show(ui, |ui| {
                    ui.set_max_width((screen.width() - 48.0).max(200.0));
                    if self.session.targets().is_empty() {
                        ui.label(RichText::new(texts::NO_SUBFOLDER).size(18.0));
                    }
                    ui.horizontal_wrapped(|ui| {
                        if let Some(parent) = self.session.parent() {
                            let label = texts::parent_button(&parent.name);
                            let resp = ui.add(egui::Button::new(RichText::new(label).size(20.0)).min_size(Vec2::new(0.0, 36.0)));
                            if resp.clicked() {
                                clicked = Some(Cmd::MoveUp);
                            }
                            if resp.has_focus() {
                                resp.surrender_focus();
                            }
                            ui.separator();
                        }
                        for (i, t) in self.session.targets().iter().enumerate() {
                            let label = match self.hotkeys[i] {
                                Some(k) => format!("[{}]  {}", k.label(), t.name),
                                None => t.name.clone(),
                            };
                            let resp = ui.add(egui::Button::new(RichText::new(label).size(20.0)).min_size(Vec2::new(0.0, 36.0)));
                            if resp.clicked() {
                                clicked = Some(Cmd::MoveTo(i));
                            }
                            // Pas de focus clavier sur les boutons : Espace/Entrée/flèches restent à l'application.
                            if resp.has_focus() {
                                resp.surrender_focus();
                            }
                        }
                    });
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(texts::HELP)
                            .size(14.0)
                            .color(Color32::from_gray(150)),
                    );
                });
            });
        clicked
    }
}
