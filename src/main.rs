// Pas de console en release : les erreurs fatales s'affichent dans une boîte de dialogue.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use imagesort::app::SorterApp;
use imagesort::session::{RecycleBin, Session};
use imagesort::settings::WindowSettings;
use std::path::PathBuf;

fn error_box(msg: &str) {
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("ImageSort")
        .set_description(msg)
        .show();
}

fn main() {
    // Dossier à trier : premier argument (ou glisser-déposer d'un dossier sur l'exe),
    // sinon sélecteur de dossier.
    let root: PathBuf = match std::env::args_os().nth(1) {
        Some(arg) => PathBuf::from(arg),
        None => match rfd::FileDialog::new().set_title("Choisir le dossier à trier").pick_folder() {
            Some(p) => p,
            None => return,
        },
    };

    let session = match Session::open(&root, Box::new(RecycleBin)) {
        Ok(s) => s,
        Err(e) => {
            error_box(&format!("Impossible d'ouvrir « {} » :\n{e}", root.display()));
            return;
        }
    };

    // Taille/position de la fenêtre du dernier lancement (réglage global).
    let settings_path = WindowSettings::default_path();
    let settings = settings_path.as_deref().map(WindowSettings::load).unwrap_or_default();

    let mut viewport = egui::ViewportBuilder::default()
        .with_title(format!("ImageSort — {}", root.display()))
        .with_min_inner_size([320.0, 240.0])
        .with_fullscreen(settings.fullscreen)
        .with_maximized(settings.maximized && !settings.fullscreen);
    // Taille et position ne s'appliquent qu'en mode fenêtré : fournies à la création d'une fenêtre
    // plein écran, elles la font s'ouvrir en fenêtre. (F11 y revient : voir `SorterApp`.)
    if !settings.fullscreen {
        if let Some((w, h)) = settings.size {
            viewport = viewport.with_inner_size([w, h]);
        }
        if let Some((x, y)) = settings.startup_position() {
            viewport = viewport.with_position([x, y]);
        }
    }
    // Icône de la fenêtre (celle de l'exe est embarquée par build.rs).
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png")) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };

    let creator: eframe::AppCreator<'_> = Box::new(move |cc| {
        let mut app = SorterApp::new(&cc.egui_ctx, session);
        if let Some(path) = settings_path {
            app.remember_window(path, settings);
        }
        Ok(Box::new(app))
    });
    if let Err(e) = eframe::run_native("ImageSort", options, creator) {
        error_box(&format!("Erreur d'affichage : {e}"));
    }
}
