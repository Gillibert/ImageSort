//! Tri « naturel » : `img2.jpg` < `img10.jpg`, insensible à la casse.

use std::cmp::Ordering;

/// Compare deux chaînes en traitant les suites de chiffres comme des nombres.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut ai = a.chars().peekable();
    let mut bi = b.chars().peekable();
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return a.cmp(b), // départage déterministe (casse)
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(ca), Some(cb)) => {
                if ca.is_ascii_digit() && cb.is_ascii_digit() {
                    let na = take_number(&mut ai);
                    let nb = take_number(&mut bi);
                    // Zéros de tête ignorés, puis longueur, puis ordre lexicographique.
                    let ta = na.trim_start_matches('0');
                    let tb = nb.trim_start_matches('0');
                    let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                    if ord != Ordering::Equal {
                        return ord;
                    }
                } else {
                    let la = ca.to_lowercase().next().unwrap_or(ca);
                    let lb = cb.to_lowercase().next().unwrap_or(cb);
                    if la != lb {
                        return la.cmp(&lb);
                    }
                    ai.next();
                    bi.next();
                }
            }
        }
    }
}

fn take_number(it: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut s = String::new();
    while let Some(&c) = it.peek() {
        if c.is_ascii_digit() {
            s.push(c);
            it.next();
        } else {
            break;
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(mut v: Vec<&str>) -> Vec<&str> {
        v.sort_by(|a, b| natural_cmp(a, b));
        v
    }

    #[test]
    fn numbers_are_compared_numerically() {
        assert_eq!(
            sorted(vec!["img10.jpg", "img2.jpg", "img1.jpg", "img100.jpg"]),
            vec!["img1.jpg", "img2.jpg", "img10.jpg", "img100.jpg"]
        );
    }

    #[test]
    fn case_insensitive() {
        assert_eq!(sorted(vec!["b", "A", "c"]), vec!["A", "b", "c"]);
    }

    #[test]
    fn leading_zeros() {
        assert_eq!(sorted(vec!["a010", "a9", "a0011"]), vec!["a9", "a010", "a0011"]);
    }

    #[test]
    fn prefix_sorts_first_and_equal_is_equal() {
        assert_eq!(natural_cmp("ab", "abc"), Ordering::Less);
        assert_eq!(natural_cmp("abc", "ab"), Ordering::Greater);
        assert_eq!(natural_cmp("same", "same"), Ordering::Equal);
    }

    #[test]
    fn huge_numbers_do_not_overflow() {
        let a = "x99999999999999999999999999";
        let b = "x100000000000000000000000000";
        assert_eq!(natural_cmp(a, b), Ordering::Less);
    }
}
