//! État d'une session de tri : liste d'images, position courante, historique d'annulation.

use crate::scan::{self, Target};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

/// Nombre d'images préchargées dans le sens de la navigation / dans le sens inverse.
pub const PREFETCH_AHEAD: usize = 8;
pub const PREFETCH_BEHIND: usize = 2;

/// Abstraction de la « corbeille » (remplaçable dans les tests).
pub trait Trash: Send {
    /// Met le fichier à la corbeille.
    fn trash(&self, path: &Path) -> io::Result<()>;
    /// Restaure à son emplacement d'origine un fichier précédemment mis à la corbeille.
    fn restore(&self, path: &Path) -> io::Result<()>;
}

/// Corbeille Windows réelle.
pub struct RecycleBin;

impl Trash for RecycleBin {
    fn trash(&self, path: &Path) -> io::Result<()> {
        trash::delete(path).map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))
    }

    fn restore(&self, path: &Path) -> io::Result<()> {
        let other = |e: trash::Error| io::Error::new(io::ErrorKind::Other, e.to_string());
        let items = trash::os_limited::list().map_err(other)?;
        // Le plus récemment supprimé parmi ceux dont l'origine est ce chemin.
        let item = items
            .into_iter()
            .filter(|i| i.original_path() == path)
            .max_by_key(|i| i.time_deleted)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "introuvable dans la corbeille"))?;
        trash::os_limited::restore_all([item]).map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))
    }
}

#[derive(Debug)]
pub enum SessionError {
    /// Plus d'image à traiter.
    NoImage,
    /// Numéro de répertoire cible inconnu.
    NoSuchTarget,
    /// Le dossier trié n'a pas de dossier parent (racine de disque).
    NoParent,
    /// Le fichier courant a disparu du disque (il est retiré de la liste).
    SourceMissing(PathBuf),
    /// Annulation impossible : l'emplacement d'origine est occupé.
    OriginOccupied(PathBuf),
    Io(io::Error),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionError::NoImage => write!(f, "aucune image"),
            SessionError::NoSuchTarget => write!(f, "répertoire inconnu"),
            SessionError::NoParent => write!(f, "pas de dossier parent"),
            SessionError::SourceMissing(p) => write!(f, "fichier disparu : {}", p.display()),
            SessionError::OriginOccupied(p) => {
                write!(f, "annulation impossible, {} existe déjà", p.display())
            }
            SessionError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for SessionError {}

impl From<io::Error> for SessionError {
    fn from(e: io::Error) -> Self {
        SessionError::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, SessionError>;

#[derive(Debug, Clone)]
enum Action {
    Moved { orig: PathBuf, dest: PathBuf, index: usize },
    Deleted { orig: PathBuf, index: usize },
}

/// Ce que `undo` vient de défaire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Undone {
    Moved { file: PathBuf, from: PathBuf },
    Deleted { file: PathBuf },
}

pub struct Session {
    root: PathBuf,
    images: Vec<PathBuf>,
    current: usize,
    targets: Vec<Target>,
    parent: Option<Target>,
    undo: Vec<Action>,
    trash: Box<dyn Trash>,
}

/// Dossier parent de `root` (chemin absolu, sans résoudre les liens), avec son nom d'affichage.
fn parent_target(root: &Path) -> Option<Target> {
    let abs = std::path::absolute(root).ok()?;
    let parent = abs.parent()?.to_path_buf();
    let name = parent.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| parent.display().to_string());
    parent.is_dir().then_some(Target { name, path: parent })
}

impl Session {
    /// Ouvre `root` : liste ses images (non récursif) et ses sous-répertoires (récursif).
    pub fn open(root: &Path, trash: Box<dyn Trash>) -> io::Result<Session> {
        let images = scan::list_images(root)?;
        let targets = scan::list_targets(root)?;
        let parent = parent_target(root);
        Ok(Session { root: root.to_path_buf(), images, current: 0, targets, parent, undo: Vec::new(), trash })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    /// Dossier parent du dossier trié, s'il existe (pas pour une racine de disque).
    pub fn parent(&self) -> Option<&Target> {
        self.parent.as_ref()
    }
    pub fn targets(&self) -> &[Target] {
        &self.targets
    }
    pub fn len(&self) -> usize {
        self.images.len()
    }
    pub fn is_empty(&self) -> bool {
        self.images.is_empty()
    }
    /// Index (base 0) de l'image courante.
    pub fn index(&self) -> usize {
        self.current
    }
    pub fn current(&self) -> Option<&Path> {
        self.images.get(self.current).map(|p| p.as_path())
    }
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    // ---------------------------------------------------------------- navigation

    /// Image suivante. `false` si on est déjà à la fin.
    pub fn next(&mut self) -> bool {
        if self.current + 1 < self.images.len() {
            self.current += 1;
            true
        } else {
            false
        }
    }

    /// Image précédente. `false` si on est déjà au début.
    pub fn prev(&mut self) -> bool {
        if self.current > 0 {
            self.current -= 1;
            true
        } else {
            false
        }
    }

    pub fn first(&mut self) {
        self.current = 0;
    }

    pub fn last(&mut self) {
        self.current = self.images.len().saturating_sub(1);
    }

    fn clamp(&mut self) {
        if self.current >= self.images.len() {
            self.current = self.images.len().saturating_sub(1);
        }
    }

    /// Images à précharger, par ordre de priorité décroissante : l'image courante,
    /// puis alternativement les suivantes (dans le sens de navigation) et les précédentes.
    pub fn prefetch_window(&self, forward: bool) -> Vec<PathBuf> {
        let n = self.images.len();
        if n == 0 {
            return Vec::new();
        }
        let (ahead, behind) = (PREFETCH_AHEAD, PREFETCH_BEHIND);
        let mut out = vec![self.images[self.current].clone()];
        for k in 1..=ahead.max(behind) {
            if k <= ahead {
                let i = if forward { self.current.checked_add(k).filter(|&i| i < n) } else { self.current.checked_sub(k) };
                if let Some(i) = i {
                    out.push(self.images[i].clone());
                }
            }
            if k <= behind {
                let i = if forward { self.current.checked_sub(k) } else { self.current.checked_add(k).filter(|&i| i < n) };
                if let Some(i) = i {
                    out.push(self.images[i].clone());
                }
            }
        }
        out
    }

    // ---------------------------------------------------------------- actions

    /// Déplace l'image courante dans le répertoire cible `target`, puis passe à la suivante.
    /// Retourne le chemin final (renommé en cas de collision de nom).
    pub fn move_to(&mut self, target: usize) -> Result<PathBuf> {
        let dir = self.targets.get(target).ok_or(SessionError::NoSuchTarget)?.path.clone();
        self.move_into(&dir)
    }

    /// Déplace l'image courante dans le dossier parent du dossier trié (`TRUC` si on trie
    /// `TRUC/Tosort`), puis passe à la suivante. L'image sort ainsi de la liste de tri.
    pub fn move_up(&mut self) -> Result<PathBuf> {
        let dir = self.parent.as_ref().ok_or(SessionError::NoParent)?.path.clone();
        self.move_into(&dir)
    }

    fn move_into(&mut self, dir: &Path) -> Result<PathBuf> {
        let src = self.images.get(self.current).ok_or(SessionError::NoImage)?.clone();
        let dest = unique_destination(dir, &src);
        if let Err(e) = move_file(&src, &dest) {
            return Err(self.fail_on_source(&src, e));
        }
        self.images.remove(self.current);
        self.undo.push(Action::Moved { orig: src, dest: dest.clone(), index: self.current });
        self.clamp();
        Ok(dest)
    }

    /// Met l'image courante à la corbeille, puis passe à la suivante.
    pub fn delete(&mut self) -> Result<PathBuf> {
        let src = self.images.get(self.current).ok_or(SessionError::NoImage)?.clone();
        if let Err(e) = self.trash.trash(&src) {
            return Err(self.fail_on_source(&src, e));
        }
        self.images.remove(self.current);
        self.undo.push(Action::Deleted { orig: src.clone(), index: self.current });
        self.clamp();
        Ok(src)
    }

    /// Si le fichier source n'existe plus, on le retire de la liste.
    fn fail_on_source(&mut self, src: &Path, e: io::Error) -> SessionError {
        if !src.exists() {
            if let Some(pos) = self.images.iter().position(|p| p == src) {
                self.images.remove(pos);
                self.clamp();
            }
            SessionError::SourceMissing(src.to_path_buf())
        } else {
            SessionError::Io(e)
        }
    }

    /// Annule la dernière action (déplacement ou suppression) : l'image revient
    /// à sa place dans la liste et devient l'image courante. `Ok(None)` si rien à annuler.
    pub fn undo(&mut self) -> Result<Option<Undone>> {
        let Some(action) = self.undo.pop() else { return Ok(None) };
        match action {
            Action::Moved { orig, dest, index } => {
                if orig.exists() {
                    self.undo.push(Action::Moved { orig: orig.clone(), dest, index });
                    return Err(SessionError::OriginOccupied(orig));
                }
                if !dest.exists() {
                    // Le fichier a été déplacé/supprimé à la main : on abandonne cette entrée.
                    return Err(SessionError::SourceMissing(dest));
                }
                if let Err(e) = move_file(&dest, &orig) {
                    self.undo.push(Action::Moved { orig, dest, index });
                    return Err(e.into());
                }
                self.reinsert(index, orig.clone());
                let from = dest.parent().map(Path::to_path_buf).unwrap_or_default();
                Ok(Some(Undone::Moved { file: orig, from }))
            }
            Action::Deleted { orig, index } => {
                if let Err(e) = self.trash.restore(&orig) {
                    self.undo.push(Action::Deleted { orig, index });
                    return Err(e.into());
                }
                self.reinsert(index, orig.clone());
                Ok(Some(Undone::Deleted { file: orig }))
            }
        }
    }

    fn reinsert(&mut self, index: usize, path: PathBuf) {
        let index = index.min(self.images.len());
        self.images.insert(index, path);
        self.current = index;
    }
}

/// Chemin libre dans `dir` pour `src` : `nom.ext`, sinon `nom_1.ext`, `nom_2.ext`…
fn unique_destination(dir: &Path, src: &Path) -> PathBuf {
    let name = src.file_name().unwrap_or_default();
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let stem = src.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    let ext = src.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (1u32..)
        .map(|n| dir.join(format!("{stem}_{n}{ext}")))
        .find(|p| !p.exists())
        .expect("espace de noms épuisé")
}

/// Déplace un fichier. Sous Windows, un décodeur de préchargement peut tenir le
/// fichier ouvert quelques millisecondes : on réessaie brièvement. Repli en
/// copie + suppression pour les déplacements entre volumes.
fn move_file(src: &Path, dest: &Path) -> io::Result<()> {
    let mut last_err = None;
    for attempt in 0..6 {
        match fs::rename(src, dest) {
            Ok(()) => return Ok(()),
            Err(e) => {
                let retryable = e.kind() == io::ErrorKind::PermissionDenied || matches!(e.raw_os_error(), Some(5 | 32 | 33));
                last_err = Some(e);
                if !retryable || !src.exists() {
                    break;
                }
                if attempt < 5 {
                    thread::sleep(Duration::from_millis(15));
                }
            }
        }
    }
    let rename_err = last_err.unwrap();
    // Repli : copie puis suppression (volumes différents).
    if src.exists() && !dest.exists() && fs::copy(src, dest).is_ok() {
        if fs::remove_file(src).is_ok() {
            return Ok(());
        }
        let _ = fs::remove_file(dest);
    }
    Err(rename_err)
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    /// « Corbeille » de test : déplace dans un dossier, mémorise l'origine.
    pub struct FolderTrash {
        pub dir: PathBuf,
        pub log: std::sync::Mutex<Vec<(PathBuf, PathBuf)>>,
    }

    impl FolderTrash {
        pub fn new(dir: PathBuf) -> Self {
            fs::create_dir_all(&dir).unwrap();
            FolderTrash { dir, log: Default::default() }
        }
    }

    impl Trash for FolderTrash {
        fn trash(&self, path: &Path) -> io::Result<()> {
            let n = self.log.lock().unwrap().len();
            let to = self.dir.join(format!("{n}_{}", path.file_name().unwrap().to_string_lossy()));
            fs::rename(path, &to)?;
            self.log.lock().unwrap().push((path.to_path_buf(), to));
            Ok(())
        }
        fn restore(&self, path: &Path) -> io::Result<()> {
            let mut log = self.log.lock().unwrap();
            let pos = log.iter().rposition(|(o, _)| o == path).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "absent"))?;
            let (_, trashed) = log.remove(pos);
            fs::rename(trashed, path)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::FolderTrash;
    use super::*;
    use std::fs::{create_dir_all, write};

    struct Fixture {
        _tmp: tempfile::TempDir,
        root: PathBuf,
        session: Session,
    }

    fn names(s: &Session) -> Vec<String> {
        s.images.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect()
    }

    /// TOSORT/{1..n}.jpg, TOSORT/Alice/Bob, TOSORT/Eve — la corbeille de test est hors de la racine.
    fn fixture(n: usize) -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("TOSORT");
        create_dir_all(root.join("Alice").join("Bob")).unwrap();
        create_dir_all(root.join("Eve")).unwrap();
        for i in 1..=n {
            write(root.join(format!("{i}.jpg")), format!("content{i}")).unwrap();
        }
        let trash = FolderTrash::new(tmp.path().join("trash"));
        let session = Session::open(&root, Box::new(trash)).unwrap();
        Fixture { _tmp: tmp, root, session }
    }

    #[test]
    fn opens_with_targets_alice_bob_eve() {
        let f = fixture(3);
        let t: Vec<&str> = f.session.targets().iter().map(|t| t.name.as_str()).collect();
        assert_eq!(t, vec!["Alice", "Alice/Bob", "Eve"]);
        assert_eq!(f.session.len(), 3);
        assert_eq!(f.session.index(), 0);
    }

    #[test]
    fn navigation_is_bounded() {
        let mut f = fixture(3);
        let s = &mut f.session;
        assert!(!s.prev());
        assert!(s.next() && s.next());
        assert!(!s.next());
        assert_eq!(s.index(), 2);
        s.first();
        assert_eq!(s.index(), 0);
        s.last();
        assert_eq!(s.index(), 2);
    }

    #[test]
    fn move_sends_file_to_target_and_shows_next() {
        let mut f = fixture(3);
        let dest = f.session.move_to(1).unwrap(); // Alice/Bob
        assert_eq!(dest, f.root.join("Alice").join("Bob").join("1.jpg"));
        assert!(dest.exists());
        assert!(!f.root.join("1.jpg").exists());
        assert_eq!(names(&f.session), vec!["2.jpg", "3.jpg"]);
        assert_eq!(f.session.current().unwrap().file_name().unwrap(), "2.jpg");
    }

    #[test]
    fn moving_the_last_image_shows_the_previous_one() {
        let mut f = fixture(3);
        f.session.last();
        f.session.move_to(2).unwrap();
        assert_eq!(f.session.current().unwrap().file_name().unwrap(), "2.jpg");
    }

    #[test]
    fn moving_everything_leaves_an_empty_session() {
        let mut f = fixture(2);
        f.session.move_to(0).unwrap();
        f.session.move_to(0).unwrap();
        assert!(f.session.is_empty());
        assert!(f.session.current().is_none());
        assert!(matches!(f.session.move_to(0), Err(SessionError::NoImage)));
        assert!(matches!(f.session.delete(), Err(SessionError::NoImage)));
    }

    #[test]
    fn unknown_target_is_rejected_without_side_effects() {
        let mut f = fixture(1);
        assert!(matches!(f.session.move_to(99), Err(SessionError::NoSuchTarget)));
        assert_eq!(f.session.len(), 1);
        assert_eq!(f.session.undo_len(), 0);
    }

    #[test]
    fn undo_restores_file_position_and_selection() {
        let mut f = fixture(4);
        f.session.next(); // sur 2.jpg
        f.session.move_to(0).unwrap(); // 2.jpg -> Alice ; courant = 3.jpg
        assert_eq!(names(&f.session), vec!["1.jpg", "3.jpg", "4.jpg"]);

        let undone = f.session.undo().unwrap().unwrap();
        assert_eq!(undone, Undone::Moved { file: f.root.join("2.jpg"), from: f.root.join("Alice") });
        assert_eq!(names(&f.session), vec!["1.jpg", "2.jpg", "3.jpg", "4.jpg"]);
        assert_eq!(f.session.current().unwrap().file_name().unwrap(), "2.jpg");
        assert!(f.root.join("2.jpg").exists());
        assert!(!f.root.join("Alice").join("2.jpg").exists());
    }

    #[test]
    fn repeated_undo_walks_back_through_history() {
        let mut f = fixture(3);
        f.session.move_to(0).unwrap(); // 1 -> Alice
        f.session.move_to(2).unwrap(); // 2 -> Eve
        f.session.move_to(1).unwrap(); // 3 -> Bob
        assert!(f.session.is_empty());

        f.session.undo().unwrap().unwrap();
        assert_eq!(names(&f.session), vec!["3.jpg"]);
        f.session.undo().unwrap().unwrap();
        assert_eq!(names(&f.session), vec!["2.jpg", "3.jpg"]);
        assert_eq!(f.session.current().unwrap().file_name().unwrap(), "2.jpg");
        f.session.undo().unwrap().unwrap();
        assert_eq!(names(&f.session), vec!["1.jpg", "2.jpg", "3.jpg"]);
        assert_eq!(f.session.undo().unwrap(), None); // plus rien à annuler
        assert!(f.root.join("1.jpg").exists() && f.root.join("2.jpg").exists() && f.root.join("3.jpg").exists());
    }

    #[test]
    fn name_collision_is_resolved_and_undo_uses_the_actual_name() {
        let mut f = fixture(1);
        write(f.root.join("Alice").join("1.jpg"), b"other").unwrap();
        let dest = f.session.move_to(0).unwrap();
        assert_eq!(dest, f.root.join("Alice").join("1_1.jpg"));
        assert_eq!(fs::read(f.root.join("Alice").join("1.jpg")).unwrap(), b"other"); // intact

        f.session.undo().unwrap().unwrap();
        assert_eq!(fs::read(f.root.join("1.jpg")).unwrap(), b"content1");
        assert_eq!(fs::read(f.root.join("Alice").join("1.jpg")).unwrap(), b"other");
        assert!(!f.root.join("Alice").join("1_1.jpg").exists());
    }

    #[test]
    fn delete_removes_image_and_undo_brings_it_back() {
        let mut f = fixture(3);
        f.session.next(); // 2.jpg
        f.session.delete().unwrap();
        assert_eq!(names(&f.session), vec!["1.jpg", "3.jpg"]);
        assert!(!f.root.join("2.jpg").exists());

        let undone = f.session.undo().unwrap().unwrap();
        assert_eq!(undone, Undone::Deleted { file: f.root.join("2.jpg") });
        assert_eq!(names(&f.session), vec!["1.jpg", "2.jpg", "3.jpg"]);
        assert_eq!(fs::read(f.root.join("2.jpg")).unwrap(), b"content2");
    }

    #[test]
    fn undo_mixes_moves_and_deletes_in_order() {
        let mut f = fixture(3);
        f.session.move_to(0).unwrap(); // 1 -> Alice
        f.session.delete().unwrap(); // 2 supprimé
        f.session.undo().unwrap().unwrap(); // 2 revient
        assert_eq!(names(&f.session), vec!["2.jpg", "3.jpg"]);
        f.session.undo().unwrap().unwrap(); // 1 revient
        assert_eq!(names(&f.session), vec!["1.jpg", "2.jpg", "3.jpg"]);
    }

    #[test]
    fn undo_refuses_to_overwrite_an_occupied_origin() {
        let mut f = fixture(1);
        f.session.move_to(0).unwrap();
        write(f.root.join("1.jpg"), b"new file").unwrap();
        assert!(matches!(f.session.undo(), Err(SessionError::OriginOccupied(_))));
        assert_eq!(f.session.undo_len(), 1); // l'entrée est conservée
        assert_eq!(fs::read(f.root.join("1.jpg")).unwrap(), b"new file");
        // Une fois la place libérée, l'annulation fonctionne.
        fs::remove_file(f.root.join("1.jpg")).unwrap();
        assert!(f.session.undo().unwrap().is_some());
    }

    #[test]
    fn file_vanishing_from_disk_is_dropped_from_the_list() {
        let mut f = fixture(3);
        fs::remove_file(f.root.join("1.jpg")).unwrap();
        assert!(matches!(f.session.move_to(0), Err(SessionError::SourceMissing(_))));
        assert_eq!(names(&f.session), vec!["2.jpg", "3.jpg"]);
        assert_eq!(f.session.undo_len(), 0);
    }

    #[test]
    fn prefetch_window_is_prioritised_and_direction_aware() {
        let mut f = fixture(20);
        for _ in 0..10 {
            f.session.next();
        }
        let name = |p: &PathBuf| p.file_stem().unwrap().to_string_lossy().into_owned();
        let fwd: Vec<String> = f.session.prefetch_window(true).iter().map(name).collect();
        // courant = 11 ; ahead 8, behind 2, alternés par distance
        assert_eq!(fwd[0], "11");
        assert_eq!(&fwd[1..5], ["12", "10", "13", "9"]);
        assert_eq!(fwd.len(), 1 + PREFETCH_AHEAD + PREFETCH_BEHIND);
        assert!(fwd.contains(&"19".to_string()) && !fwd.contains(&"8".to_string()));

        let back: Vec<String> = f.session.prefetch_window(false).iter().map(name).collect();
        assert_eq!(&back[..3], ["11", "10", "12"]);
        assert!(back.contains(&"3".to_string()) && !back.contains(&"14".to_string()));
    }

    #[test]
    fn prefetch_window_at_the_edges() {
        let mut f = fixture(3);
        assert_eq!(f.session.prefetch_window(true).len(), 3);
        f.session.last();
        assert_eq!(f.session.prefetch_window(true).len(), 3);
        let e = fixture(0);
        assert!(e.session.prefetch_window(true).is_empty());
    }

    #[test]
    fn move_file_creates_nothing_extra_in_root() {
        let mut f = fixture(2);
        f.session.move_to(2).unwrap();
        let remaining: Vec<_> = fs::read_dir(&f.root).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert!(remaining.contains(&"2.jpg".to_string()));
        assert!(!remaining.contains(&"1.jpg".to_string()));
    }

    #[test]
    fn parent_is_the_folder_above_the_sorted_one() {
        let f = fixture(1);
        let parent = f.session.parent().expect("a parent");
        assert_eq!(parent.path, f.root.parent().unwrap());
        assert_eq!(parent.name, f.root.parent().unwrap().file_name().unwrap().to_string_lossy());
    }

    #[test]
    fn move_up_sends_the_image_to_the_parent_and_shows_the_next_one() {
        let mut f = fixture(3);
        let up = f.root.parent().unwrap().to_path_buf();
        let dest = f.session.move_up().unwrap();
        assert_eq!(dest, up.join("1.jpg"));
        assert_eq!(fs::read_to_string(&dest).unwrap(), "content1");
        assert!(!f.root.join("1.jpg").exists());
        assert_eq!(names(&f.session), vec!["2.jpg", "3.jpg"]);
        assert_eq!(f.session.current().unwrap().file_name().unwrap(), "2.jpg");
    }

    #[test]
    fn move_up_can_be_undone() {
        let mut f = fixture(2);
        let up = f.root.parent().unwrap().to_path_buf();
        f.session.move_up().unwrap();
        let undone = f.session.undo().unwrap().unwrap();
        assert!(matches!(undone, Undone::Moved { .. }));
        assert!(f.root.join("1.jpg").exists());
        assert!(!up.join("1.jpg").exists());
        assert_eq!(names(&f.session), vec!["1.jpg", "2.jpg"]);
    }

    #[test]
    fn move_up_never_overwrites_a_file_of_the_parent() {
        let mut f = fixture(1);
        let up = f.root.parent().unwrap().to_path_buf();
        write(up.join("1.jpg"), "already there").unwrap();
        let dest = f.session.move_up().unwrap();
        assert_eq!(dest, up.join("1_1.jpg"));
        assert_eq!(fs::read_to_string(up.join("1.jpg")).unwrap(), "already there");
    }

    #[test]
    fn move_up_with_no_image_is_an_error() {
        let mut f = fixture(0);
        assert!(matches!(f.session.move_up(), Err(SessionError::NoImage)));
    }

    #[test]
    fn move_up_without_parent_is_an_error() {
        let mut f = fixture(1);
        f.session.parent = None;
        assert!(matches!(f.session.move_up(), Err(SessionError::NoParent)));
        assert!(f.root.join("1.jpg").exists());
    }

    #[test]
    fn parent_of_a_relative_path_and_of_a_drive_root() {
        let cwd = std::env::current_dir().unwrap();
        let rel = parent_target(Path::new("some_folder")).expect("parent of a relative path");
        assert_eq!(rel.path, cwd);
        let dot = parent_target(Path::new(".")).expect("parent of the current folder");
        assert_eq!(dot.path, cwd.parent().unwrap());
        #[cfg(windows)]
        assert!(parent_target(Path::new("C:\\")).is_none());
    }
}
