//! Reading shell text (package.json scripts, Makefile recipes, Taskfile commands) into
//! the programs it runs. Shared by toolchains; knows nothing about any of them.

/// The shell's own words: builtins and keywords, not programs.
const BUILTINS: [&str; 35] = [
    "cd", "export", "set", "unset", "source", ".", ":", "eval", "fi", "for", "done", "case", "esac", "[", "[[", "}", "test", "true",
    "false", "echo", "printf", "exit", "return", "shift", "wait", "trap", "read", "ulimit", "umask", "pushd", "popd", "declare",
    "local", "alias", "type",
];

/// Words before a command that aren't it: `then foo`, `time foo`, `exec foo`.
const PREFIXES: [&str; 12] = ["if", "then", "else", "elif", "while", "until", "do", "!", "{", "time", "exec", "command"];

/// The programs a script runs, each once: the first word of each command (after
/// keywords, variable assignments and redirections), not the shell's own words, paths
/// (`./build.sh`) nor expansions. Quotes are honoured: `node -e "a && b"` is one command.
pub fn programs(script: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for words in commands(script) {
        let mut words = words.iter().map(String::as_str).peekable();
        let program = loop {
            let Some(w) = words.next() else { break None };
            if PREFIXES.contains(&w) || is_assignment(w) {
                continue;
            }
            if let Some(target) = redirection(w) {
                // `2> log`: the next word is where it goes.
                if target.is_empty() {
                    words.next();
                }
                continue;
            }
            break Some(w);
        };
        let Some(p) = program else { continue };
        if p.is_empty() || p.contains(['/', '$', '`']) || BUILTINS.contains(&p) || out.iter().any(|o| o == p) {
            continue;
        }
        out.push(p.to_string());
    }
    out
}

/// A script's commands as words, split on `&&`, `||`, `|`, `;`, `&`, newlines and
/// parentheses outside quotes; quotes removed. `>&` and `&>` are redirections, not
/// separators.
fn commands(script: &str) -> Vec<Vec<String>> {
    let (mut all, mut words, mut word) = (Vec::new(), Vec::new(), String::new());
    let mut quote: Option<char> = None;
    let chars: Vec<char> = script.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') => {
                i += 1;
                word.extend(chars.get(i));
            }
            (Some(_), c) => word.push(c),
            (None, '\'' | '"') => quote = Some(c),
            (None, '\\') => {
                i += 1;
                word.extend(chars.get(i));
            }
            (None, '&') if word.ends_with(['>', '<']) || chars.get(i + 1) == Some(&'>') => word.push(c),
            (None, '&' | '|' | ';' | '\n' | '(' | ')') => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
                if !words.is_empty() {
                    all.push(std::mem::take(&mut words));
                }
            }
            (None, c) if c.is_whitespace() => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            }
            (None, c) => word.push(c),
        }
        i += 1;
    }
    if !word.is_empty() {
        words.push(word);
    }
    if !words.is_empty() {
        all.push(words);
    }
    all
}

/// `NAME=value` before a command.
fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// For a redirection (`>log`, `2>&1`, `&>`, `<in`, `>>`), what follows its operator
/// (empty when the target is the next word).
fn redirection(word: &str) -> Option<&str> {
    let rest = word.trim_start_matches(|c: char| c.is_ascii_digit());
    let rest = rest.strip_prefix('&').filter(|r| r.starts_with('>')).unwrap_or(rest);
    if !rest.starts_with(['>', '<']) {
        return None;
    }
    Some(rest.trim_start_matches(['>', '<', '&']))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn programs_honour_quotes_redirections_and_keywords() {
        let p = |s: &str| programs(s);
        assert_eq!(p("tsc 2>&1 | tee log"), ["tsc", "tee"], "2>&1 isn't a separator");
        assert_eq!(p("vite build >&2 && eslint . &> out.txt"), ["vite", "eslint"]);
        assert_eq!(p("jest --testPathPattern='(a|b)'"), ["jest"], "quoted separators");
        assert_eq!(p(r#"node -e "x && y" && grep -E "a|b" f"#), ["node", "grep"]);
        assert_eq!(p(r#"FOO="a b" BAR=1 vite"#), ["vite"], "quoted assignments");
        assert_eq!(p("if [ -d dist ]; then rimraf dist; fi"), ["rimraf"], "after then");
        assert_eq!(p("ulimit -n 4096; read x; : noop; > log vite"), ["vite"], "builtins and a leading redirection");
        assert_eq!(p("cross-env NODE_ENV=production webpack"), ["cross-env"]);
        assert_eq!(p("time exec tsx watch src"), ["tsx"]);
        assert_eq!(p("./gen.sh; $TOOL x; echo done; vite; vite"), ["vite"], "paths, expansions, builtins; once each");
    }
}
