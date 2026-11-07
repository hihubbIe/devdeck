//! Fuzzy matching for the palette (R5.3): the query's characters must appear in order;
//! consecutive runs and word starts score higher. Ties sort alphabetically.

/// Characters that start a word after them.
fn is_separator(c: char) -> bool {
    matches!(c, ' ' | '/' | '-' | '_' | ':' | '.' | '›' | '·' | '@')
}

/// The score of `text` for `query`, `None` when it doesn't match. Case-insensitive;
/// spaces in the query are ignored.
pub fn score(query: &str, text: &str) -> Option<i32> {
    let q: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect();
    if q.is_empty() {
        return Some(0);
    }
    let t: Vec<char> = text.chars().collect();
    let lower: Vec<char> = t.iter().map(|c| c.to_lowercase().next().unwrap_or(*c)).collect();
    let mut qi = 0;
    let mut total = 0;
    let mut last: Option<usize> = None;
    for (i, c) in lower.iter().enumerate() {
        if qi < q.len() && *c == q[qi] {
            let mut s = 1;
            if last == Some(i.wrapping_sub(1)) {
                s += 5;
            }
            if i == 0 || is_separator(t[i - 1]) {
                s += 8;
            }
            total += s;
            last = Some(i);
            qi += 1;
        }
    }
    // Shorter texts win among equals.
    (qi == q.len()).then(|| total * 100 - t.len() as i32)
}

/// Sorts `items` best first by `score` of `key(item)`, dropping non-matches; ties keep
/// alphabetical order of the key.
pub fn rank<T>(query: &str, items: Vec<T>, key: impl Fn(&T) -> &str) -> Vec<T> {
    let mut scored: Vec<(i32, T)> = items.into_iter().filter_map(|it| score(query, key(&it)).map(|s| (s, it))).collect();
    scored.sort_by(|(a, x), (b, y)| b.cmp(a).then_with(|| key(x).to_lowercase().cmp(&key(y).to_lowercase())));
    scored.into_iter().map(|(_, it)| it).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_matters_case_doesnt() {
        assert!(score("ensvc", "EntityService").is_some());
        assert!(score("svcen", "entityservice").is_none());
        assert_eq!(score("", "anything"), Some(0));
    }

    #[test]
    fn word_starts_and_runs_win() {
        let list = vec!["mastertadaservice", "entityservice", "eventservice", "front-end/mastertadaapp"];
        let ranked = rank("es", list.clone(), |s| s);
        assert_eq!(ranked[0], "eventservice", "e…s: word start then a later s; shorter wins the tie");
        let ranked = rank("mta", list.clone(), |s| s);
        assert_eq!(ranked[0], "mastertadaservice");
        let ranked = rank("fe/m", list, |s| s);
        assert_eq!(ranked, ["front-end/mastertadaapp"]);
    }

    #[test]
    fn ties_are_alphabetical() {
        let ranked = rank("", vec!["b", "a", "c"], |s| s);
        assert_eq!(ranked, ["a", "b", "c"]);
    }
}
