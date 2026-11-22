//! Text shown to the human, in their language. The strings live in `locales/<code>.json`, shared
//! with the UI; a key missing from a language falls back to English, then to the key itself.
//! Text for the agents is not translated: it stays English.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

include!(concat!(env!("OUT_DIR"), "/locales.rs"));

pub const FALLBACK: &str = "en";

type Table = HashMap<String, String>;

fn tables() -> &'static HashMap<&'static str, Table> {
    static T: OnceLock<HashMap<&'static str, Table>> = OnceLock::new();
    T.get_or_init(|| LOCALES.iter().map(|(code, json)| (*code, serde_json::from_str(json).unwrap_or_else(|e| panic!("locales/{code}.json: {e}")))).collect())
}

fn current() -> &'static RwLock<String> {
    static C: OnceLock<RwLock<String>> = OnceLock::new();
    C.get_or_init(|| RwLock::new(FALLBACK.into()))
}

/// The best available language for a list of wanted ones (BCP 47 or POSIX, best first):
/// an exact match, else the same base language, else English.
pub fn pick<'a>(wanted: impl IntoIterator<Item = &'a str>) -> String {
    let have: Vec<&str> = LOCALES.iter().map(|(c, _)| *c).collect();
    let norm = |s: &str| s.split('.').next().unwrap_or("").replace('_', "-").to_lowercase();
    for w in wanted {
        let w = norm(w);
        if let Some(c) = have.iter().find(|c| c.to_lowercase() == w) {
            return c.to_string();
        }
        let base = w.split('-').next().unwrap_or("");
        if let Some(c) = have.iter().find(|c| !base.is_empty() && c.to_lowercase().split('-').next() == Some(base)) {
            return c.to_string();
        }
    }
    FALLBACK.into()
}

/// The system's language, from the usual environment variables; on macOS from the system
/// setting, since apps started from the Dock or Finder get no LANG.
fn system() -> String {
    let mut wanted: Vec<String> = ["LC_ALL", "LC_MESSAGES", "LANG"].iter().filter_map(|v| std::env::var(v).ok()).filter(|v| !v.is_empty() && v != "C" && v != "POSIX").collect();
    if cfg!(target_os = "macos") {
        wanted.splice(0..0, mac_languages());
    }
    pick(wanted.iter().map(String::as_str))
}

/// `defaults read -g AppleLanguages`: the languages picked in System Settings, best first
/// (e.g. "zh-Hant-TW", "en-US").
fn mac_languages() -> Vec<String> {
    let Ok(out) = std::process::Command::new("defaults").args(["read", "-g", "AppleLanguages"]).output() else { return vec![] };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim().trim_matches(|c| c == '"' || c == ',' || c == '(' || c == ')').to_string())
        .filter(|l| !l.is_empty())
        // "zh-Hant-TW" → also try "zh-TW", since locales are named by region.
        .flat_map(|l| {
            let parts: Vec<&str> = l.split('-').collect();
            let short = (parts.len() == 3).then(|| format!("{}-{}", parts[0], parts[2]));
            short.into_iter().chain(std::iter::once(l.clone()))
        })
        .collect()
}

/// `None` follows the system.
pub fn set_lang(lang: Option<&str>) {
    let code = match lang {
        Some(l) => pick([l]),
        None => system(),
    };
    *current().write().unwrap() = code;
}

/// The text for `key` in the current language.
pub fn t(key: &str) -> String {
    let lang = current().read().unwrap().clone();
    let tabs = tables();
    tabs.get(lang.as_str()).and_then(|t| t.get(key)).or_else(|| tabs.get(FALLBACK).and_then(|t| t.get(key))).cloned().unwrap_or_else(|| key.to_string())
}

/// `tr!("agent.joined", alias = name)`: the text with each `{alias}` filled in.
macro_rules! tr {
    ($key:expr) => { $crate::i18n::t($key) };
    ($key:expr, $($name:ident = $val:expr),+ $(,)?) => {{
        let mut s = $crate::i18n::t($key);
        $( s = s.replace(concat!("{", stringify!($name), "}"), &$val.to_string()); )+
        s
    }};
}
pub(crate) use tr;

#[cfg(test)]
mod tests {
    use super::*;

    /// Every built-in language has every key, with the same `{placeholders}`. `key_one` (the
    /// singular, for languages that have one) is optional.
    #[test]
    fn languages_are_complete() {
        let en = &tables()[FALLBACK];
        let holes = |s: &str| {
            let mut v: Vec<String> = s.split('{').skip(1).filter_map(|p| p.split_once('}').map(|(n, _)| n.to_string())).collect();
            v.sort();
            v
        };
        for (code, table) in tables() {
            assert!(table.contains_key("_name"), "{code}: no _name");
            for (k, v) in en.iter().filter(|(k, _)| !k.ends_with("_one")) {
                let other = table.get(k).unwrap_or_else(|| panic!("{code} is missing {k}"));
                assert_eq!(holes(v), holes(other), "{code}: {k} has different placeholders");
            }
            for k in table.keys().filter(|k| !k.ends_with("_one")) {
                assert!(en.contains_key(k), "{code} has {k}, which en lacks");
            }
        }
    }

    #[test]
    fn picks_the_closest_language() {
        assert_eq!(pick(["zh_TW.UTF-8"]), "zh-TW");
        assert_eq!(pick(["zh-HK"]), "zh-TW");
        assert_eq!(pick(["fr-FR", "en-US"]), "en");
        assert_eq!(pick(["xx"]), "en");
        assert_eq!(pick(["zh-TW", "zh-Hant-TW"]), "zh-TW");
    }
}
