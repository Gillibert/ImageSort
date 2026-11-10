// Pas de console en release : les erreurs fatales s'affichent dans une boîte de dialogue.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use imagesort::app::SorterApp;
use imagesort::session::{RecycleBin, Session};
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

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(format!("ImageSort — {}", root.display()))
            .with_fullscreen(true),
        ..Default::default()
    };

    if let Err(e) = eframe::run_native("ImageSort", options, Box::new(move |cc| Ok(Box::new(SorterApp::new(&cc.egui_ctx, session))))) {
        error_box(&format!("Erreur d'affichage : {e}"));
    }
}
