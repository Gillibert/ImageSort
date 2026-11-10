//! Attribution des touches aux répertoires cibles.
//!
//! Les lettres A..Y sont utilisées dans l'ordre (Z est réservée à l'annulation),
//! puis F1..F10 (F11 = plein écran) si l'on a plus de 25 répertoires. Au-delà, les répertoires
//! restent accessibles à la souris uniquement.

/// Une touche attribuée à un répertoire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hotkey {
    Letter(char),
    Function(u8),
}

impl Hotkey {
    /// Libellé affiché sur le bouton.
    pub fn label(&self) -> String {
        match self {
            Hotkey::Letter(c) => c.to_string(),
            Hotkey::Function(n) => format!("F{n}"),
        }
    }
}

const LETTERS: &str = "ABCDEFGHIJKLMNOPQRSTUVWXY"; // sans Z
const MAX_FUNCTION_KEYS: u8 = 10;

/// Touche du `index`-ième répertoire (0 = premier), s'il en reste une.
pub fn hotkey_for(index: usize) -> Option<Hotkey> {
    let letters = LETTERS.len();
    if index < letters {
        LETTERS.chars().nth(index).map(Hotkey::Letter)
    } else if index < letters + MAX_FUNCTION_KEYS as usize {
        Some(Hotkey::Function((index - letters) as u8 + 1))
    } else {
        None
    }
}

/// Touches pour `count` répertoires.
pub fn assign(count: usize) -> Vec<Option<Hotkey>> {
    (0..count).map(hotkey_for).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn first_keys_are_a_b_c() {
        assert_eq!(hotkey_for(0), Some(Hotkey::Letter('A')));
        assert_eq!(hotkey_for(1), Some(Hotkey::Letter('B')));
        assert_eq!(hotkey_for(2), Some(Hotkey::Letter('C')));
    }

    #[test]
    fn z_is_never_assigned() {
        let all = assign(100);
        assert!(!all.contains(&Some(Hotkey::Letter('Z'))));
        assert_eq!(hotkey_for(24), Some(Hotkey::Letter('Y')));
        assert_eq!(hotkey_for(25), Some(Hotkey::Function(1)));
    }

    #[test]
    fn keys_are_unique_and_run_out() {
        let all = assign(60);
        let used: Vec<_> = all.iter().flatten().collect();
        assert_eq!(used.len(), 25 + 10);
        let set: HashSet<String> = used.iter().map(|k| k.label()).collect();
        assert_eq!(set.len(), used.len());
        assert_eq!(all[34], Some(Hotkey::Function(10)));
        assert_eq!(all[35], None);
    }

    #[test]
    fn labels() {
        assert_eq!(Hotkey::Letter('E').label(), "E");
        assert_eq!(Hotkey::Function(3).label(), "F3");
    }
}

