//! Test d'intégration « sans écran » : on pilote l'application egui avec des
//! événements clavier synthétiques et on vérifie le disque et le préchargement.

use egui::{Event, Key, Modifiers, RawInput, Rect};
use image::{Rgb, RgbImage};
use imagesort::app::SorterApp;
use imagesort::session::{Session, Trash};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Corbeille de test : un dossier hors de la racine triée.
struct FolderTrash {
    dir: PathBuf,
    log: Mutex<Vec<(PathBuf, PathBuf)>>,
}

impl Trash for FolderTrash {
    fn trash(&self, path: &Path) -> io::Result<()> {
        let mut log = self.log.lock().unwrap();
        let to = self.dir.join(format!("{}_{}", log.len(), path.file_name().unwrap().to_string_lossy()));
        fs::rename(path, &to)?;
        log.push((path.to_path_buf(), to));
        Ok(())
    }
    fn restore(&self, path: &Path) -> io::Result<()> {
        let mut log = self.log.lock().unwrap();
        let pos = log.iter().rposition(|(o, _)| o == path).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "absent"))?;
        let (_, trashed) = log.remove(pos);
        fs::rename(trashed, path)
    }
}

struct Harness {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    ctx: egui::Context,
    app: SorterApp,
}

impl Harness {
    /// TOSORT/{img01..imgN}.{png,jpg,webp}, TOSORT/Alice/Bob, TOSORT/Eve
    fn new(n: usize) -> Harness {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("TOSORT");
        fs::create_dir_all(root.join("Alice").join("Bob")).unwrap();
        fs::create_dir_all(root.join("Eve")).unwrap();
        for i in 1..=n {
            let img = RgbImage::from_pixel(1600, 1200, Rgb([(i * 20 % 256) as u8, 80, 160]));
            let ext = ["png", "jpg", "webp"][i % 3];
            img.save(root.join(format!("img{i:02}.{ext}"))).unwrap();
        }
        let trash = FolderTrash { dir: tmp.path().join("trash"), log: Default::default() };
        fs::create_dir_all(&trash.dir).unwrap();
        let session = Session::open(&root, Box::new(trash)).unwrap();
        let ctx = egui::Context::default();
        let app = SorterApp::new(&ctx, session);
        let mut h = Harness { _tmp: tmp, root, ctx, app };
        h.frame(vec![]); // premier cadre : lance le préchargement
        h
    }

    fn frame(&mut self, events: Vec<Event>) {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1920.0, 1080.0))),
            events,
            ..Default::default()
        };
        let app = &mut self.app;
        let mut output = self.ctx.run_ui(raw, |ui| {
            let ctx = ui.ctx().clone();
            app.step(&ctx);
            app.paint(ui);
        });
        // Pas de GPU ici : on jette les envois de textures (sinon egui s'en plaint à la destruction).
        output.textures_delta.clear();
    }

    fn key_event(key: Key, pressed: bool) -> Event {
        // `repeat` est recalculé par egui d'après l'état « touche enfoncée ».
        Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers: Modifiers::NONE }
    }

    /// Appui bref : enfoncée puis relâchée.
    fn press(&mut self, key: Key) {
        self.frame(vec![Self::key_event(key, true), Self::key_event(key, false)]);
    }

    /// Touche maintenue pendant `frames` cadres (le premier est l'appui, les suivants des répétitions).
    fn hold(&mut self, key: Key, frames: usize) {
        for _ in 0..frames {
            self.frame(vec![Self::key_event(key, true)]);
        }
    }

    fn release(&mut self, key: Key) {
        self.frame(vec![Self::key_event(key, false)]);
    }

    /// Fait tourner des cadres jusqu'à ce que `cond` soit vraie.
    fn wait_until(&mut self, what: &str, cond: impl Fn(&SorterApp) -> bool) {
        let start = Instant::now();
        while !cond(&self.app) {
            assert!(start.elapsed() < Duration::from_secs(20), "délai dépassé : {what}");
            std::thread::sleep(Duration::from_millis(5));
            self.frame(vec![]);
        }
    }

    fn names(&self, dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().unwrap().is_file())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    fn current_stem(&self) -> String {
        self.app.session().current().map(|p| p.file_stem().unwrap().to_string_lossy().into_owned()).unwrap_or_default()
    }
}

#[test]
fn letters_move_images_and_z_undoes_them_one_by_one() {
    let mut h = Harness::new(6);
    h.wait_until("première image", |a| a.current_ready());
    assert_eq!(h.current_stem(), "img01");

    // Alice = A, Alice/Bob = B, Eve = C
    h.press(Key::A);
    assert_eq!(h.current_stem(), "img02");
    h.press(Key::B);
    h.press(Key::C);
    assert_eq!(h.current_stem(), "img04");

    assert_eq!(h.names(&h.root.join("Alice")), vec!["img01.jpg"]);
    assert_eq!(h.names(&h.root.join("Alice").join("Bob")), vec!["img02.webp"]);
    assert_eq!(h.names(&h.root.join("Eve")), vec!["img03.png"]);
    assert_eq!(h.app.session().len(), 3);

    // Z annule dans l'ordre inverse : Eve, puis Bob, puis Alice.
    h.press(Key::Z);
    assert_eq!(h.current_stem(), "img03");
    assert!(h.names(&h.root.join("Eve")).is_empty());
    h.press(Key::Z);
    assert_eq!(h.current_stem(), "img02");
    h.press(Key::Z);
    assert_eq!(h.current_stem(), "img01");
    assert_eq!(h.app.session().len(), 6);
    assert_eq!(h.names(&h.root).len(), 6);
    assert!(h.names(&h.root.join("Alice")).is_empty());

    // Plus rien à annuler : message d'erreur, aucun effet.
    h.press(Key::Z);
    assert_eq!(h.app.status_text(), Some("Rien à annuler"));
    assert_eq!(h.app.session().len(), 6);
}

#[test]
fn arrows_navigate_without_touching_the_disk() {
    let mut h = Harness::new(4);
    h.press(Key::ArrowRight);
    h.press(Key::ArrowRight);
    assert_eq!(h.current_stem(), "img03");
    h.press(Key::ArrowLeft);
    assert_eq!(h.current_stem(), "img02");
    h.press(Key::End);
    assert_eq!(h.current_stem(), "img04");
    h.press(Key::ArrowRight); // déjà à la fin
    assert_eq!(h.current_stem(), "img04");
    h.press(Key::Home);
    assert_eq!(h.current_stem(), "img01");
    assert_eq!(h.names(&h.root).len(), 4);
}

#[test]
fn delete_key_trashes_and_z_restores() {
    let mut h = Harness::new(3);
    h.press(Key::ArrowRight);
    let victim = h.app.session().current().unwrap().to_path_buf();
    h.press(Key::Delete);
    assert!(!victim.exists());
    assert_eq!(h.app.session().len(), 2);
    assert_eq!(h.current_stem(), "img03");
    h.press(Key::Z);
    assert!(victim.exists());
    assert_eq!(h.current_stem(), "img02");
    assert_eq!(h.app.session().len(), 3);
}

#[test]
fn holding_a_key_never_moves_or_deletes_more_than_one_image() {
    let mut h = Harness::new(6);
    // Touche maintenue : egui voit un appui, puis des appuis sans relâchement (= répétitions).
    h.hold(Key::A, 5);
    assert_eq!(h.app.session().len(), 5, "A maintenue ne doit déplacer qu'une image");
    assert_eq!(h.names(&h.root.join("Alice")).len(), 1);
    h.release(Key::A);

    h.hold(Key::Delete, 5);
    assert_eq!(h.app.session().len(), 4, "Suppr maintenue ne doit supprimer qu'une image");
    h.release(Key::Delete);

    // La navigation, elle, accepte la répétition.
    assert_eq!(h.current_stem(), "img03");
    h.hold(Key::ArrowRight, 3);
    assert_eq!(h.current_stem(), "img06");
}

#[test]
fn keys_with_ctrl_or_alt_are_ignored() {
    let mut h = Harness::new(2);
    let ev = |m: Modifiers| Event::Key { key: Key::A, physical_key: Some(Key::A), pressed: true, repeat: false, modifiers: m };
    h.frame(vec![ev(Modifiers::CTRL)]);
    h.frame(vec![ev(Modifiers::ALT)]);
    assert_eq!(h.app.session().len(), 2);
}

#[test]
fn unassigned_letters_do_nothing() {
    let mut h = Harness::new(2);
    h.press(Key::Q); // seulement 3 répertoires : A, B, C
    h.press(Key::X);
    assert_eq!(h.app.session().len(), 2);
    assert_eq!(h.names(&h.root).len(), 2);
}

#[test]
fn next_images_are_ready_before_they_are_needed() {
    let mut h = Harness::new(12);
    // On laisse le préchargement se terminer.
    h.wait_until("préchargement", |a| a.cached_count() >= 9);

    // Parcours rapide : chaque image est déjà décodée au moment où elle s'affiche.
    for _ in 0..3 {
        h.press(Key::ArrowRight);
        assert!(h.app.current_ready(), "image {} pas prête", h.current_stem());
    }
    // Après un tri, l'image suivante est elle aussi déjà prête.
    h.press(Key::A);
    assert!(h.app.current_ready(), "image {} pas prête après tri", h.current_stem());
    h.press(Key::B);
    assert!(h.app.current_ready());
}

#[test]
fn cache_stays_bounded_on_large_folders() {
    let mut h = Harness::new(40);
    h.wait_until("préchargement", |a| a.cached_count() >= 9);
    for _ in 0..20 {
        h.press(Key::ArrowRight);
        std::thread::sleep(Duration::from_millis(5));
    }
    h.wait_until("préchargement", |a| a.current_ready());
    h.frame(vec![]);
    // 1 courante + 8 devant + 2 derrière au maximum.
    assert!(h.app.cached_count() <= 11, "cache trop gros : {}", h.app.cached_count());
}

#[test]
fn empty_folder_and_missing_subfolders_do_not_crash() {
    let mut h = Harness::new(0);
    h.press(Key::ArrowRight);
    h.press(Key::A);
    h.press(Key::Delete);
    h.press(Key::Z);
    assert!(h.app.session().is_empty());
}

#[test]
fn avif_images_are_listed_displayed_and_sorted() {
    let mut h = Harness::new(0);
    let avif = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("data").join("gradient.avif");
    fs::copy(&avif, h.root.join("photo.avif")).unwrap();
    let trash = FolderTrash { dir: h.root.parent().unwrap().join("trash"), log: Default::default() };
    h.app = SorterApp::new(&h.ctx, Session::open(&h.root, Box::new(trash)).unwrap());
    assert_eq!(h.app.session().len(), 1);
    h.wait_until("décodage AVIF", |a| a.current_ready());
    h.press(Key::A);
    assert_eq!(h.names(&h.root.join("Alice")), vec!["photo.avif"]);
}

#[test]
fn corrupt_image_can_still_be_sorted() {
    let mut h = Harness::new(2);
    fs::write(h.root.join("img00.jpg"), b"pas une image").unwrap();
    // Réouverture pour que la liste prenne en compte le fichier corrompu.
    let trash = FolderTrash { dir: h.root.parent().unwrap().join("trash"), log: Default::default() };
    let session = Session::open(&h.root, Box::new(trash)).unwrap();
    h.app = SorterApp::new(&h.ctx, session);
    h.wait_until("chargement", |a| a.current_ready());
    assert_eq!(h.current_stem(), "img00");
    h.press(Key::C); // Eve
    assert!(h.names(&h.root.join("Eve")).contains(&"img00.jpg".to_string()));
}

