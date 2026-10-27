//! The interface in other languages. The English text is the key: `t("Play")`
//! looks it up in `assets/lang/<code>.tsv` (lines of English, a tab, the
//! translation) and falls back to the English when a line is missing.

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

/// Code and the language's own name, in menu order. English first.
pub const LANGS: &[(&str, &str)] = &[
    ("en", "English"),
    ("de", "Deutsch"),
    ("es", "Español"),
    ("fr", "Français"),
    ("it", "Italiano"),
    ("pt-BR", "Português (Brasil)"),
    ("ja", "日本語"),
    ("zh-CN", "简体中文"),
];

const FILES: [&str; 8] = [
    "",
    include_str!("../../../assets/lang/de.tsv"),
    include_str!("../../../assets/lang/es.tsv"),
    include_str!("../../../assets/lang/fr.tsv"),
    include_str!("../../../assets/lang/it.tsv"),
    include_str!("../../../assets/lang/pt-BR.tsv"),
    include_str!("../../../assets/lang/ja.tsv"),
    include_str!("../../../assets/lang/zh-CN.tsv"),
];

static CURRENT: AtomicUsize = AtomicUsize::new(0);

/// English → translation for language `i` of `LANGS`.
pub fn table(i: usize) -> &'static HashMap<String, String> {
    static T: OnceLock<Vec<HashMap<String, String>>> = OnceLock::new();
    let unescape = |s: &str| s.replace("\\n", "\n");
    &T.get_or_init(|| {
        FILES
            .iter()
            .map(|f| f.lines().filter(|l| !l.starts_with('#')).filter_map(|l| l.split_once('\t')).map(|(k, v)| (unescape(k), unescape(v))).collect())
            .collect()
    })[i]
}

/// Uses language `code` (one of `LANGS`; anything else is English).
pub fn set(code: &str) {
    CURRENT.store(LANGS.iter().position(|l| l.0 == code).unwrap_or(0), Ordering::Relaxed);
}

/// The language in use.
pub fn code() -> &'static str {
    LANGS[CURRENT.load(Ordering::Relaxed)].0
}

/// The best of `LANGS` for a system locale like "pt-BR", "de_AT" or "zh-Hans-CN".
pub fn matching(locale: &str) -> &'static str {
    let lang = locale.split(['-', '_', '.']).next().unwrap_or("").to_ascii_lowercase();
    LANGS.iter().find(|l| l.0.split('-').next() == Some(lang.as_str())).map_or("en", |l| l.0)
}

/// `s` in the language in use.
pub fn t(s: &str) -> &str {
    match CURRENT.load(Ordering::Relaxed) {
        0 => s,
        i => table(i).get(s).map_or(s, String::as_str),
    }
}

/// `s` in the language in use, with each `{}` (or `{0}`, `{1}`…) filled in.
pub fn tf(s: &str, args: &[&dyn Display]) -> String {
    fill(t(s), args)
}

/// `s` with each `{}` (or `{0}`, `{1}`…) filled in.
pub fn fill(s: &str, args: &[&dyn Display]) -> String {
    let mut out = s.to_owned();
    for (i, a) in args.iter().enumerate() {
        out = out.replace(&format!("{{{i}}}"), &a.to_string());
    }
    for a in args {
        if let Some(at) = out.find("{}") {
            out.replace_range(at..at + 2, &a.to_string());
        }
    }
    out
}

/// For the main window's little display, whose skin font has only A to Z:
/// `s` translated in languages written with those letters (accents are
/// shown without them), in English otherwise.
pub fn tm(s: &str) -> &str {
    if matches!(code(), "ja" | "zh-CN") { s } else { t(s) }
}

/// `tm` with `{}` filled in.
pub fn tmf(s: &str, args: &[&dyn Display]) -> String {
    fill(tm(s), args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locales_and_placeholders() {
        assert_eq!(matching("pt-BR"), "pt-BR");
        assert_eq!(matching("pt_PT"), "pt-BR");
        assert_eq!(matching("de_AT.UTF-8"), "de");
        assert_eq!(matching("zh-Hans-CN"), "zh-CN");
        assert_eq!(matching("ko-KR"), "en");
        assert_eq!(fill("{} of {}", &[&1, &5]), "1 of 5");
        assert_eq!(fill("{1}の{0}", &[&"a", &"b"]), "bのa");
    }

    /// Every text the code looks up by hand (`t("…")`, `say_f("…")`…) has a line.
    #[test]
    fn every_lookup_is_translated() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
        let mut missing = Vec::new();
        for dir in ["crates/gamp-ui/src", "apps/gamp/src"] {
            for f in std::fs::read_dir(format!("{root}/{dir}")).unwrap().flatten() {
                let src = std::fs::read_to_string(f.path()).unwrap();
                let src = src.split("mod tests {").next().unwrap();
                for call in ["t(\"", "tf(\"", "tm(\"", "tmf(\"", "say(\"", "say_f(\""] {
                    for (at, _) in src.match_indices(call) {
                        let before = src[..at].chars().last().unwrap_or(' ');
                        if before.is_alphanumeric() || before == '_' {
                            continue;
                        }
                        let rest = &src[at + call.len()..];
                        let key = &rest[..rest.find('"').unwrap()];
                        if !key.is_empty() && !table(1).contains_key(key) {
                            missing.push(format!("{}: {key}", f.file_name().to_string_lossy()));
                        }
                    }
                }
            }
        }
        assert!(missing.is_empty(), "not in the translations: {missing:#?}");
    }

    /// Every translation keeps its line's `{}`s, and no language lacks a line another has.
    #[test]
    fn tables_line_up() {
        let keys = |i: usize| table(i).keys().cloned().collect::<std::collections::BTreeSet<_>>();
        let marks = |s: &str| s.matches("{}").count() + (0..9).filter(|i| s.contains(&format!("{{{i}}}"))).count();
        for i in 1..LANGS.len() {
            assert_eq!(keys(i), keys(1), "{} and {} have different lines", LANGS[i].0, LANGS[1].0);
            for (k, v) in table(i) {
                assert_eq!(marks(k), marks(v), "{}: {k:?} → {v:?}", LANGS[i].0);
            }
        }
    }
}
