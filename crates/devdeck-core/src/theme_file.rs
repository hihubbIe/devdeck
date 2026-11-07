//! Theme files (R9.2): a name and color tokens. The built-in `devdeck-dark` is embedded;
//! user themes are `<config>/themes/*.toml`. A token a file leaves out falls back to
//! devdeck-dark's; a bad value or file is reported and skipped, never fatal.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

pub const BUILTIN_DARK: &str = include_str!("../../../assets/themes/devdeck-dark.toml");

/// Every UI color token a theme defines, in display order.
pub const TOKENS: [&str; 24] = [
    "bg", "bar", "raised", "hover", "selected", "line", "line_soft", "fg", "hi", "mute", "dim", "accent", "accent_bg",
    "drop", "ok", "warn", "err", "env", "env_bg", "backdrop", "focus", "selection", "caret", "scrollbar",
];

/// Terminal colors (`[ansi]` table), for command output from M2 on.
pub const ANSI_TOKENS: [&str; 20] = [
    "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white", "bright_black", "bright_red", "bright_green",
    "bright_yellow", "bright_blue", "bright_magenta", "bright_cyan", "bright_white", "foreground", "background", "cursor",
    "selection",
];

/// A color as 0xRRGGBBAA.
pub type Rgba = u32;

#[derive(Clone, Debug, PartialEq)]
pub struct ThemeColors {
    pub name: String,
    pub colors: BTreeMap<String, Rgba>,
    pub ansi: BTreeMap<String, Rgba>,
}

impl ThemeColors {
    /// A UI color; magenta marks a token no theme defines (a bug, never user data).
    pub fn get(&self, token: &str) -> Rgba {
        self.colors.get(token).copied().unwrap_or(0xff00ffff)
    }

    pub fn ansi(&self, token: &str) -> Rgba {
        self.ansi.get(token).copied().unwrap_or(0xff00ffff)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTheme {
    name: Option<String>,
    #[serde(default)]
    colors: BTreeMap<String, String>,
    #[serde(default)]
    ansi: BTreeMap<String, String>,
}

pub fn parse_hex(s: &str) -> Option<Rgba> {
    let h = s.trim().strip_prefix('#')?;
    let v = u32::from_str_radix(h, 16).ok()?;
    match h.len() {
        6 => Some((v << 8) | 0xff),
        8 => Some(v),
        _ => None,
    }
}

/// Parses a theme file over `base`. Returns the theme and the problems found.
pub fn parse(text: &str, fallback_name: &str, base: &ThemeColors) -> Result<(ThemeColors, Vec<String>), String> {
    let raw: RawTheme = toml::from_str(text).map_err(|e| e.to_string().lines().next().unwrap_or("invalid TOML").to_string())?;
    let mut out = base.clone();
    out.name = raw.name.unwrap_or_else(|| fallback_name.to_string());
    let mut problems = Vec::new();
    for (table, raw_map, known) in [("colors", raw.colors, &TOKENS[..]), ("ansi", raw.ansi, &ANSI_TOKENS[..])] {
        for (k, v) in raw_map {
            if !known.contains(&k.as_str()) {
                problems.push(format!("unknown {table} token `{k}`"));
                continue;
            }
            match parse_hex(&v) {
                Some(c) => {
                    let target = if table == "colors" { &mut out.colors } else { &mut out.ansi };
                    target.insert(k, c);
                }
                None => problems.push(format!("`{k}` = \"{v}\" is not #rrggbb or #rrggbbaa")),
            }
        }
    }
    Ok((out, problems))
}

pub fn builtin_dark() -> ThemeColors {
    let empty = ThemeColors { name: String::new(), colors: BTreeMap::new(), ansi: BTreeMap::new() };
    let (t, problems) = parse(BUILTIN_DARK, "devdeck-dark", &empty).expect("built-in theme parses");
    debug_assert!(problems.is_empty(), "{problems:?}");
    t
}

/// All themes: the built-in one, then `<dir>/*.toml` sorted by name. Problems are
/// returned as messages for the settings view.
pub fn load_all(dir: &Path) -> (Vec<ThemeColors>, Vec<String>) {
    let dark = builtin_dark();
    let mut themes = vec![dark.clone()];
    let mut problems = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return (themes, problems) };
    let mut files: Vec<_> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "toml")).collect();
    files.sort();
    for path in files {
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("theme").to_string();
        let shown = path.display().to_string();
        match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| parse(&t, &stem, &dark)) {
            Ok((theme, _)) if theme.name == dark.name => {
                problems.push(format!("{shown}: the name `{}` is reserved for the built-in theme (skipped)", dark.name));
            }
            Ok((theme, issues)) => {
                problems.extend(issues.into_iter().map(|i| format!("{shown}: {i}")));
                themes.retain(|t| t.name != theme.name);
                themes.push(theme);
            }
            Err(e) => problems.push(format!("{shown}: {e} (skipped)")),
        }
    }
    themes[1..].sort_by(|a, b| a.name.cmp(&b.name));
    (themes, problems)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_has_every_token() {
        let t = builtin_dark();
        assert_eq!(t.name, "devdeck-dark");
        for k in TOKENS {
            assert!(t.colors.contains_key(k), "missing {k}");
        }
        for k in ANSI_TOKENS {
            assert!(t.ansi.contains_key(k), "missing ansi {k}");
        }
        assert_eq!(t.get("bg"), 0x0f1012ff);
        assert_eq!(t.get("drop"), 0x64a6e32e);
    }

    #[test]
    fn hex_parsing() {
        assert_eq!(parse_hex("#abcdef"), Some(0xabcdefff));
        assert_eq!(parse_hex("#ABCDEF80"), Some(0xabcdef80));
        assert_eq!(parse_hex("abcdef"), None);
        assert_eq!(parse_hex("#abc"), None);
        assert_eq!(parse_hex("#zzzzzz"), None);
    }

    #[test]
    fn partial_theme_falls_back_and_reports_problems() {
        let base = builtin_dark();
        let (t, problems) = parse("name = \"mine\"\n[colors]\nbg = \"#000000\"\nfg = \"white\"\nsparkle = \"#ffffff\"\n", "x", &base).unwrap();
        assert_eq!(t.name, "mine");
        assert_eq!(t.get("bg"), 0x000000ff);
        assert_eq!(t.get("fg"), base.get("fg"), "bad value keeps the fallback");
        assert_eq!(problems.len(), 2);
    }

    #[test]
    fn broken_file_is_an_error_and_name_defaults_to_file_stem() {
        let base = builtin_dark();
        assert!(parse("colors = [", "x", &base).is_err());
        assert!(parse("unknown_table = 1", "x", &base).is_err(), "unknown top-level keys are rejected");
        let (t, _) = parse("[colors]\n", "solarized", &base).unwrap();
        assert_eq!(t.name, "solarized");
    }

    #[test]
    fn load_all_from_a_folder() {
        let d = crate::testing::TempDir::new("themes");
        std::fs::write(d.join("zeta.toml"), "[colors]\nbg = \"#111111\"\n").unwrap();
        std::fs::write(d.join("alpha.toml"), "[colors]\nbg = \"#222222\"\n").unwrap();
        std::fs::write(d.join("broken.toml"), "[colors\n").unwrap();
        std::fs::write(d.join("impostor.toml"), "name = \"devdeck-dark\"\n").unwrap();
        std::fs::write(d.join("notes.txt"), "ignored").unwrap();
        let (themes, problems) = load_all(&d);
        let names: Vec<_> = themes.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["devdeck-dark", "alpha", "zeta"], "built-in first, then alphabetical");
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("broken.toml")));
        assert!(problems.iter().any(|p| p.contains("reserved")));
        assert_eq!(load_all(&d.join("missing")).0.len(), 1);
    }
}
