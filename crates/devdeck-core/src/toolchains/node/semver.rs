//! npm's version ranges (`engines.node`): `>=18`, `<21`, `^18.17`, `~20.1`, `20.x`, `20`,
//! `*`, hyphen ranges (`18 - 20`) and alternatives joined by `||`. A prerelease tag is
//! read as its version (`>=18.0.0-0` as `>=18.0.0`).

/// Whether `version` (one to three parts) fits `range`; `None` when the range can't be
/// read (`lts/*`, `>=18,<20`).
pub fn satisfies(version: &[u64], range: &str) -> Option<bool> {
    let v = [0, 1, 2].map(|i| version.get(i).copied().unwrap_or(0));
    let mut fits = false;
    for alt in range.split("||") {
        fits |= alternative(&v, alt.trim())?;
    }
    Some(fits)
}

/// One `||` alternative: comparators that must all hold, or a hyphen range.
fn alternative(v: &[u64; 3], alt: &str) -> Option<bool> {
    if alt.is_empty() || alt == "*" || alt == "x" {
        return Some(true);
    }
    if let Some((lo, hi)) = alt.split_once(" - ") {
        return Some(compare(v, ">=", lo.trim())? && compare(v, "<=", hi.trim())?);
    }
    // `>= 18` is one comparator.
    let mut comparators: Vec<String> = Vec::new();
    for token in alt.split_whitespace() {
        match comparators.last_mut() {
            Some(last) if last.chars().all(|c| "<>=~^".contains(c)) => last.push_str(token),
            _ => comparators.push(token.to_string()),
        }
    }
    let mut all = true;
    for c in &comparators {
        let op_len = c.find(|ch: char| !"<>=~^".contains(ch)).unwrap_or(c.len());
        all &= compare(v, &c[..op_len], &c[op_len..])?;
    }
    Some(all)
}

/// `v op partial`, with npm's meaning for partial versions (`>1.2` is `>=1.3.0`);
/// `None` when `partial` or `op` can't be read.
fn compare(v: &[u64; 3], op: &str, partial: &str) -> Option<bool> {
    let partial = partial.strip_prefix('v').unwrap_or(partial);
    let partial = partial.split(['-', '+']).next().unwrap_or_default();
    let mut given: Vec<u64> = Vec::new();
    for p in partial.split('.').take(3) {
        match p {
            "x" | "X" | "*" | "" => break,
            p => given.push(p.parse().ok()?),
        }
    }
    let n = given.len();
    let low = [0, 1, 2].map(|i| given.get(i).copied().unwrap_or(0));
    // Just above every version starting with the first `upto` parts (`1.2` → `1.3.0`).
    let above = |upto: usize| {
        let mut a = [0; 3];
        a[..upto].copy_from_slice(&low[..upto]);
        if upto > 0 {
            a[upto - 1] += 1;
        }
        a
    };
    Some(match op {
        "" | "=" => v[..n] == low[..n],
        ">=" => *v >= low,
        ">" if n == 3 => *v > low,
        ">" => n == 0 || *v >= above(n),
        "<" => *v < low,
        "<=" if n == 3 => *v <= low,
        "<=" => n == 0 || *v < above(n),
        "~" if n <= 1 => v[..n] == low[..n],
        "~" => *v >= low && *v < above(2),
        "^" if n == 0 => true,
        "^" => {
            let upto = if low[0] > 0 || n == 1 { 1 } else if low[1] > 0 || n == 2 { 2 } else { 3 };
            *v >= low && *v < above(upto)
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fits(v: &str, r: &str) -> Option<bool> {
        let parts: Vec<u64> = v.split('.').map(|p| p.parse().unwrap()).collect();
        satisfies(&parts, r)
    }

    #[test]
    fn ranges_as_npm_reads_them() {
        for (v, r) in [
            ("22.1.0", ">=22.0.0"),
            ("22.1.0", ">= 22"),
            ("20.11.1", "^20.10"),
            ("20.11.1", "~20.11.0"),
            ("20.11.1", "20.x"),
            ("20.11.1", "20"),
            ("18.20.4", "^18.17.0 || >=20"),
            ("20.11.1", ">=18 <21"),
            ("20.11.1", "18 - 20"),
            ("0.2.5", "^0.2.3"),
            ("1.3.0", ">1.2"),
            ("26.0.0", "*"),
            ("18.0.0", ">=18.0.0-0"),
            ("20.1.0", ">=20.0.0-rc.1"),
        ] {
            assert_eq!(fits(v, r), Some(true), "{v} fits {r}");
        }
        for (v, r) in [
            ("20.20.2", ">=22.0.0"),
            ("21.0.0", "^20.10"),
            ("20.12.0", "~20.11.0"),
            ("21.0.0", ">=18 <21"),
            ("21.0.0", "18 - 20"),
            ("0.3.0", "^0.2.3"),
            ("1.2.9", ">1.2"),
        ] {
            assert_eq!(fits(v, r), Some(false), "{v} doesn't fit {r}");
        }
    }

    #[test]
    fn unreadable_ranges_say_so() {
        for r in ["lts/*", ">=18,<20", "latest", "=>18"] {
            assert_eq!(fits("20.0.0", r), None, "{r}");
        }
    }
}
