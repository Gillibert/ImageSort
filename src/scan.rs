//! Découverte des images à trier et des répertoires cibles.

use crate::formats::is_image_path;
use crate::natural::natural_cmp;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Un répertoire de destination (un bouton dans l'interface).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// Chemin relatif à la racine, séparé par `/` (ex. `Alice/Bob`).
    pub name: String,
    /// Chemin complet.
    pub path: PathBuf,
}

/// Images situées directement dans `root` (pas dans les sous-répertoires :
/// celles-ci sont considérées comme déjà triées), en ordre naturel.
pub fn list_images(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut images = Vec::new();
    for entry in fs::read_dir(root)? {
        let Ok(entry) = entry else { continue };
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_file() {
            continue;
        }
        let path = entry.path();
        if is_image_path(&path) {
            images.push(path);
        }
    }
    images.sort_by(|a, b| {
        natural_cmp(
            &a.file_name().unwrap_or_default().to_string_lossy(),
            &b.file_name().unwrap_or_default().to_string_lossy(),
        )
    });
    Ok(images)
}

fn is_ignored_dir(name: &str) -> bool {
    name.starts_with('.') || name.starts_with('$') || name.eq_ignore_ascii_case("System Volume Information")
}

/// Tous les sous-répertoires de `root`, récursivement (Alice, Alice/Bob, Eve…),
/// triés en ordre naturel. Les répertoires cachés/système sont ignorés et les
/// liens symboliques ne sont pas suivis.
pub fn list_targets(root: &Path) -> io::Result<Vec<Target>> {
    let mut out = Vec::new();
    collect(root, "", &mut out)?;
    out.sort_by(|a, b| natural_cmp(&a.name, &b.name));
    Ok(out)
}

fn collect(dir: &Path, prefix: &str, out: &mut Vec<Target>) -> io::Result<()> {
    let rd = match fs::read_dir(dir) {
        Ok(rd) => rd,
        // Le répertoire racine doit être lisible ; un sous-répertoire illisible est ignoré.
        Err(e) if !prefix.is_empty() => {
            let _ = e;
            return Ok(());
        }
        Err(e) => return Err(e),
    };
    for entry in rd {
        let Ok(entry) = entry else { continue };
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_dir() {
            continue;
        }
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        if is_ignored_dir(&name) {
            continue;
        }
        let rel = if prefix.is_empty() { name.to_string() } else { format!("{prefix}/{name}") };
        out.push(Target { name: rel.clone(), path: entry.path() });
        collect(&entry.path(), &rel, out)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{create_dir_all, write};

    #[test]
    fn lists_only_root_images_in_natural_order() {
        let tmp = tempfile::tempdir().unwrap();
        let r = tmp.path();
        for n in ["b10.jpg", "b2.PNG", "a.webp", "notes.txt", "c.gif"] {
            write(r.join(n), b"x").unwrap();
        }
        create_dir_all(r.join("Alice")).unwrap();
        write(r.join("Alice").join("already.jpg"), b"x").unwrap();

        let names: Vec<String> = list_images(r)
            .unwrap()
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["a.webp", "b2.PNG", "b10.jpg", "c.gif"]);
    }

    #[test]
    fn lists_nested_targets_and_skips_hidden() {
        let tmp = tempfile::tempdir().unwrap();
        let r = tmp.path();
        create_dir_all(r.join("Alice").join("Bob")).unwrap();
        create_dir_all(r.join("Eve")).unwrap();
        create_dir_all(r.join(".git")).unwrap();
        create_dir_all(r.join("$RECYCLE.BIN")).unwrap();
        write(r.join("file.jpg"), b"x").unwrap();

        let t = list_targets(r).unwrap();
        let names: Vec<&str> = t.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["Alice", "Alice/Bob", "Eve"]);
        assert_eq!(t[1].path, r.join("Alice").join("Bob"));
    }

    #[test]
    fn missing_root_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("nope");
        assert!(list_images(&missing).is_err());
        assert!(list_targets(&missing).is_err());
    }
}
