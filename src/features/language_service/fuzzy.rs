//! Fuzzy matching for completion lists and pickers: the typed text must appear
//! in the candidate in order, and matches that start words, follow a separator or
//! run together score higher, so `hm` finds `HashMap` before `hashmap_len`.
use std::cmp::Reverse;

/// A match: how good it is (higher is better) and where the typed characters
/// landed, as character indices into the candidate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    pub score: i32,
    pub positions: Vec<usize>,
}

const BASE: i32 = 16;
const CONSECUTIVE: i32 = 18;
const WORD_START: i32 = 24;
const AFTER_SEPARATOR: i32 = 20;
const CAMEL: i32 = 20;
const FIRST_CHAR: i32 = 12;
const EXACT_CASE: i32 = 3;
const GAP: i32 = -2;
const LEADING_GAP: i32 = -1;

fn is_separator(c: char) -> bool {
    matches!(
        c,
        '_' | '-' | '.' | ':' | '/' | '\\' | ' ' | '$' | '#' | '<' | '('
    )
}

/// Matches `pattern` against `candidate`. An empty pattern matches everything
/// with a score of zero. The match is case-insensitive unless the pattern
/// contains an uppercase letter, which then must match exactly ("smart case").
pub fn fuzzy(pattern: &str, candidate: &str) -> Option<Match> {
    let pattern: Vec<char> = pattern.chars().collect();
    if pattern.is_empty() {
        return Some(Match {
            score: 0,
            positions: Vec::new(),
        });
    }
    let chars: Vec<char> = candidate.chars().collect();
    if pattern.len() > chars.len() {
        return None;
    }
    let smart_case = pattern.iter().any(|c| c.is_uppercase());
    let same = |p: char, c: char| {
        if smart_case {
            p == c
        } else {
            p.to_lowercase().eq(c.to_lowercase())
        }
    };
    // Bonus for matching at each candidate position, from its surroundings.
    let boundary: Vec<i32> = (0..chars.len())
        .map(|i| {
            let c = chars[i];
            if i == 0 {
                return WORD_START + FIRST_CHAR;
            }
            let before = chars[i - 1];
            if is_separator(before) {
                AFTER_SEPARATOR
            } else if before.is_lowercase() && c.is_uppercase() {
                CAMEL
            } else if !before.is_alphanumeric() && c.is_alphanumeric() {
                AFTER_SEPARATOR
            } else if before.is_alphabetic() != c.is_alphabetic() && c.is_alphanumeric() {
                CAMEL / 2
            } else {
                0
            }
        })
        .collect();

    // Dynamic programming over (pattern index, candidate index): the best score
    // with pattern[..=p] matched and pattern[p] placed at candidate position c.
    let (n, m) = (pattern.len(), chars.len());
    const NONE: i32 = i32::MIN;
    let mut best = vec![vec![NONE; m]; n];
    let mut back = vec![vec![usize::MAX; m]; n];
    for c in 0..m {
        if same(pattern[0], chars[c]) {
            best[0][c] = BASE
                + boundary[c]
                + LEADING_GAP * c as i32
                + if chars[c] == pattern[0] {
                    EXACT_CASE
                } else {
                    0
                };
        }
    }
    for p in 1..n {
        // The best earlier cell, kept as a running maximum that pays for the gap.
        let mut running: (i32, usize) = (NONE, usize::MAX);
        for c in p..m {
            // The cell at c-1 becomes available as a predecessor for c.
            let previous = best[p - 1][c - 1];
            if previous != NONE {
                if running.0 != NONE {
                    running.0 += GAP;
                }
                if running.0 == NONE || previous > running.0 {
                    running = (previous, c - 1);
                }
            } else if running.0 != NONE {
                running.0 += GAP;
            }
            if !same(pattern[p], chars[c]) {
                continue;
            }
            let mut options: [(i32, usize); 2] = [(NONE, usize::MAX); 2];
            if previous != NONE {
                options[0] = (previous + CONSECUTIVE, c - 1);
            }
            if running.0 != NONE && running.1 + 1 != c {
                options[1] = (running.0, running.1);
            }
            let (score, from) = if options[0].0 >= options[1].0 {
                options[0]
            } else {
                options[1]
            };
            if score == NONE {
                continue;
            }
            best[p][c] = score
                + BASE
                + boundary[c]
                + if chars[c] == pattern[p] {
                    EXACT_CASE
                } else {
                    0
                };
            back[p][c] = from;
        }
    }
    let (score, end) = (0..m)
        .filter(|&c| best[n - 1][c] != NONE)
        .map(|c| (best[n - 1][c], c))
        .max_by_key(|&(score, c)| (score, Reverse(c)))?;
    let mut positions = vec![0; n];
    let mut at = end;
    for p in (0..n).rev() {
        positions[p] = at;
        if p > 0 {
            at = back[p][at];
        }
    }
    // Shorter candidates win ties: a closer match to what was typed.
    Some(Match {
        score: score - (m - n) as i32 / 4,
        positions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(pattern: &str, candidate: &str) -> Option<i32> {
        fuzzy(pattern, candidate).map(|m| m.score)
    }

    #[test]
    fn the_pattern_must_appear_in_order() {
        assert!(fuzzy("hm", "HashMap").is_some());
        assert!(fuzzy("mh", "HashMap").is_none());
        assert!(fuzzy("xyz", "HashMap").is_none());
        assert!(
            fuzzy("hashmapp", "HashMap").is_none(),
            "longer than the candidate"
        );
        let empty = fuzzy("", "anything").unwrap();
        assert_eq!((empty.score, empty.positions.len()), (0, 0));
    }

    #[test]
    fn positions_name_the_matched_characters() {
        assert_eq!(fuzzy("hm", "HashMap").unwrap().positions, [0, 4]);
        assert_eq!(fuzzy("gfn", "get_file_name").unwrap().positions, [0, 4, 9]);
        assert_eq!(fuzzy("abc", "xabcx").unwrap().positions, [1, 2, 3]);
    }

    #[test]
    fn word_starts_and_runs_beat_scattered_matches() {
        assert!(score("hm", "HashMap") > score("hm", "hammer_mat"));
        assert!(score("len", "len") > score("len", "plenty"));
        assert!(score("fo", "foo") > score("fo", "bar_fx_o"));
        assert!(score("gfn", "get_file_name") > score("gfn", "gofunction"));
        assert!(score("print", "println") > score("print", "sprint"));
    }

    #[test]
    fn smart_case_only_cares_when_the_pattern_has_capitals() {
        assert!(fuzzy("map", "HashMap").is_some());
        assert!(fuzzy("Map", "HashMap").is_some());
        assert!(fuzzy("Map", "hashmap").is_none());
        assert!(score("Hash", "HashMap") > score("hash", "HashMap"));
    }

    #[test]
    fn a_shorter_candidate_wins_when_nothing_else_differs() {
        assert!(score("ab", "ab") > score("ab", "abcdefgh"));
    }

    #[test]
    fn unicode_candidates_are_matched_by_character() {
        let found = fuzzy("zh", "变量zhong").unwrap();
        assert_eq!(found.positions, [2, 3]);
    }
}
