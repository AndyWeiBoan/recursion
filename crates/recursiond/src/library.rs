//! Recursion's own Skills and rules: copied in from where the human keeps them for Claude and
//! Codex (or for Cursor, for rules), then given to the agents a project, chat or agent picks.
//! A copy, not a link: what is imported stays as it was when imported, and shows "changed" when
//! the original moves on (import it again to take the new one).
//!
//! The agents see only these: what the human put in the CLIs' own folders is turned off for them
//! (`claude_own_skills`, `codex_own_skills`); what the CLIs come with stays.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Skill,
    Rule,
}

/// One imported Skill or rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    /// Where it was copied from.
    pub from: String,
    /// When (ms since epoch).
    pub at: u64,
    /// Of what was copied, to tell when the original changed.
    pub hash: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Meta {
    #[serde(default)]
    skills: BTreeMap<String, Entry>,
    #[serde(default)]
    rules: BTreeMap<String, Entry>,
}

/// Something found on this computer that can be imported.
#[derive(Debug, Clone, Serialize)]
pub struct Found {
    pub name: String,
    pub path: String,
    /// Whose folder it is in: "claude", "codex" or "cursor".
    pub vendor: String,
    pub description: String,
    /// Imported already, the same / imported, but the original changed since / not yet.
    pub state: &'static str,
}

/// An entry of the library, for the settings page and the pickers.
#[derive(Debug, Clone, Serialize)]
pub struct Item {
    pub name: String,
    pub from: String,
    pub at: u64,
    pub description: String,
    /// A rule's "only when working on these files" (Claude's `paths`, Cursor's `globs`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    /// The original changed since it was imported.
    pub changed: bool,
    /// The original is gone.
    pub missing: bool,
}

fn root() -> PathBuf {
    crate::home()
}
fn dir(kind: Kind) -> PathBuf {
    root().join(match kind {
        Kind::Skill => "skills",
        Kind::Rule => "rules",
    })
}
fn meta_file() -> PathBuf {
    root().join("library.json")
}
fn load() -> Meta {
    std::fs::read_to_string(meta_file()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}
fn save(m: &Meta) -> Result<(), String> {
    let s = serde_json::to_string_pretty(m).map_err(|e| e.to_string())?;
    std::fs::write(meta_file(), s).map_err(|e| e.to_string())
}
fn entries(m: &mut Meta, kind: Kind) -> &mut BTreeMap<String, Entry> {
    match kind {
        Kind::Skill => &mut m.skills,
        Kind::Rule => &mut m.rules,
    }
}

/// A Skill's folder (in the library) or a rule's file.
pub fn path_of(kind: Kind, name: &str) -> PathBuf {
    match kind {
        Kind::Skill => dir(kind).join(name),
        Kind::Rule => dir(kind).join(format!("{name}.md")),
    }
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_default()
}
/// Claude's folder (`CLAUDE_CONFIG_DIR`, else ~/.claude) and Codex's (`CODEX_HOME`, else ~/.codex).
pub fn claude_dir(account: Option<PathBuf>) -> PathBuf {
    account.or_else(|| std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from)).unwrap_or_else(|| home().join(".claude"))
}
pub fn codex_dir(account: Option<PathBuf>) -> PathBuf {
    account.or_else(|| std::env::var_os("CODEX_HOME").map(PathBuf::from)).unwrap_or_else(|| home().join(".codex"))
}

/// The frontmatter's fields we read: `name`, `description`, and a rule's file patterns.
struct Front {
    name: Option<String>,
    description: String,
    paths: Vec<String>,
    body: String,
}
fn front(text: &str) -> Front {
    let mut f = Front { name: None, description: String::new(), paths: vec![], body: text.to_string() };
    let Some(rest) = text.strip_prefix("---\n").or_else(|| text.strip_prefix("---\r\n")) else { return f };
    let Some(end) = rest.find("\n---") else { return f };
    let head = &rest[..end];
    f.body = rest[end + 4..].trim_start_matches(['-', '\r', '\n']).to_string();
    let lines: Vec<&str> = head.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i];
        let (k, v) = l.split_once(':').map(|(k, v)| (k.trim(), v.trim())).unwrap_or(("", ""));
        // A value on the lines below (a folded string or a list), indented.
        let mut more = vec![];
        while i + 1 < lines.len() && (lines[i + 1].starts_with(' ') || lines[i + 1].starts_with('\t')) {
            more.push(lines[i + 1].trim());
            i += 1;
        }
        let unquote = |s: &str| s.trim().trim_matches(['"', '\'']).to_string();
        match k {
            "name" => f.name = Some(unquote(v)).filter(|s| !s.is_empty()),
            "description" => f.description = if v.is_empty() || v == ">" || v == "|" || v == ">-" { more.join(" ") } else { unquote(v) },
            "paths" | "globs" => {
                let mut p: Vec<String> = more.iter().filter_map(|m| m.strip_prefix('-')).map(unquote).collect();
                if p.is_empty() {
                    p = v.trim_matches(['[', ']']).split(',').map(unquote).filter(|s| !s.is_empty()).collect();
                }
                f.paths = p;
            }
            _ => {}
        }
        i += 1;
    }
    f
}

/// Of a file or folder's contents, to tell a changed original.
fn hash(p: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let mut files = vec![];
    collect(p, p, &mut files);
    files.sort();
    for (rel, full) in files {
        rel.hash(&mut h);
        std::fs::read(full).unwrap_or_default().hash(&mut h);
    }
    format!("{:016x}", h.finish())
}
fn collect(base: &Path, p: &Path, out: &mut Vec<(String, PathBuf)>) {
    if p.is_file() {
        out.push((p.strip_prefix(base).unwrap_or(p).to_string_lossy().to_string(), p.to_path_buf()));
    } else if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            collect(base, &e.path(), out);
        }
    }
}
fn copy(from: &Path, to: &Path) -> std::io::Result<()> {
    if from.is_file() {
        if let Some(d) = to.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::copy(from, to).map(|_| ())
    } else {
        std::fs::create_dir_all(to)?;
        for e in std::fs::read_dir(from)?.flatten() {
            copy(&e.path(), &to.join(e.file_name()))?;
        }
        Ok(())
    }
}

fn skill_dirs_in(d: &Path, vendor: &str, out: &mut Vec<(PathBuf, String)>) {
    let Ok(rd) = std::fs::read_dir(d) else { return };
    for e in rd.flatten() {
        let p = e.path();
        // Codex's own come with it (.system): not the human's.
        if p.join("SKILL.md").is_file() && !e.file_name().to_string_lossy().starts_with('.') {
            out.push((p, vendor.to_string()));
        }
    }
}
fn rule_files_in(d: &Path, vendor: &str, out: &mut Vec<(PathBuf, String)>) {
    let Ok(rd) = std::fs::read_dir(d) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            rule_files_in(&p, vendor, out);
        } else if p.extension().is_some_and(|x| x == "md" || x == "mdc") {
            out.push((p, vendor.to_string()));
        }
    }
}

/// The Skills the human put where Claude reads them (its folder, and the project's).
fn claude_skills(claude: Option<&Path>, cwd: Option<&str>) -> Vec<(PathBuf, String)> {
    let mut out = vec![];
    if let Some(d) = claude {
        skill_dirs_in(&d.join("skills"), "claude", &mut out);
    }
    if let Some(c) = cwd {
        skill_dirs_in(&Path::new(c).join(".claude/skills"), "claude", &mut out);
    }
    out
}
/// The Skills the human put where Codex reads them (its folder and ~/.agents, and the project's).
fn codex_skills(codex: Option<&Path>, cwd: Option<&str>) -> Vec<(PathBuf, String)> {
    let mut out = vec![];
    if let Some(d) = codex {
        skill_dirs_in(&d.join("skills"), "codex", &mut out);
        skill_dirs_in(&home().join(".agents/skills"), "codex", &mut out);
    }
    if let Some(c) = cwd {
        skill_dirs_in(&Path::new(c).join(".agents/skills"), "codex", &mut out);
        skill_dirs_in(&Path::new(c).join(".codex/skills"), "codex", &mut out);
    }
    out
}
fn skill_name(p: &Path) -> String {
    let text = std::fs::read_to_string(p.join("SKILL.md")).unwrap_or_default();
    front(&text).name.unwrap_or_else(|| p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())
}

/// What Claude would load of the human's own, to turn off for an agent: names for `skillOverrides`.
pub fn claude_own_skills(claude: &Path, cwd: &str) -> Vec<String> {
    let mut n: Vec<String> = claude_skills(Some(claude), Some(cwd)).iter().map(|(p, _)| skill_name(p)).collect();
    n.sort();
    n.dedup();
    n
}
/// What Codex would load of the human's own, to turn off for an agent: their SKILL.md files.
pub fn codex_own_skills(codex: &Path, cwd: &str) -> Vec<PathBuf> {
    codex_skills(Some(codex), Some(cwd)).into_iter().map(|(p, _)| p.join("SKILL.md")).collect()
}

/// Everything importable on this computer: the CLIs' folders, and those of the projects open.
pub fn scan(kind: Kind, projects: &[String]) -> Vec<Found> {
    let mut raw: Vec<(PathBuf, String)> = vec![];
    let (claude, codex) = (claude_dir(None), codex_dir(None));
    match kind {
        Kind::Skill => {
            raw.extend(claude_skills(Some(&claude), None));
            raw.extend(codex_skills(Some(&codex), None));
            for c in projects {
                raw.extend(claude_skills(None, Some(c)));
                raw.extend(codex_skills(None, Some(c)));
            }
        }
        Kind::Rule => {
            rule_files_in(&claude.join("rules"), "claude", &mut raw);
            for c in projects {
                rule_files_in(&Path::new(c).join(".claude/rules"), "claude", &mut raw);
                rule_files_in(&Path::new(c).join(".cursor/rules"), "cursor", &mut raw);
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    raw.retain(|(p, _)| seen.insert(p.clone()));
    let mut meta = load();
    let lib = entries(&mut meta, kind).clone();
    let mut out: Vec<Found> = raw
        .into_iter()
        .map(|(p, vendor)| {
            let (name, description) = match kind {
                Kind::Skill => {
                    let f = front(&std::fs::read_to_string(p.join("SKILL.md")).unwrap_or_default());
                    (f.name.unwrap_or_else(|| p.file_name().unwrap_or_default().to_string_lossy().to_string()), f.description)
                }
                Kind::Rule => {
                    let f = front(&std::fs::read_to_string(&p).unwrap_or_default());
                    (p.file_stem().unwrap_or_default().to_string_lossy().to_string(), if f.description.is_empty() { first_line(&f.body) } else { f.description })
                }
            };
            let path = p.to_string_lossy().to_string();
            let state = match lib.values().find(|e| e.from == path) {
                Some(e) if e.hash == hash(&p) => "same",
                Some(_) => "changed",
                None => "new",
            };
            Found { name, path, vendor, description, state }
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name).then(a.path.cmp(&b.path)));
    out
}

fn first_line(s: &str) -> String {
    s.lines().map(|l| l.trim().trim_start_matches('#').trim()).find(|l| !l.is_empty()).unwrap_or("").chars().take(120).collect()
}

/// What is in the library, each told whether its original changed or is gone.
pub fn list(kind: Kind) -> Vec<Item> {
    let mut meta = load();
    entries(&mut meta, kind)
        .iter()
        .map(|(name, e)| {
            let text = match kind {
                Kind::Skill => std::fs::read_to_string(path_of(kind, name).join("SKILL.md")).unwrap_or_default(),
                Kind::Rule => std::fs::read_to_string(path_of(kind, name)).unwrap_or_default(),
            };
            let f = front(&text);
            let src = Path::new(&e.from);
            Item {
                name: name.clone(),
                from: e.from.clone(),
                at: e.at,
                description: if f.description.is_empty() { first_line(&f.body) } else { f.description },
                paths: f.paths,
                changed: src.exists() && hash(src) != e.hash,
                missing: !src.exists(),
            }
        })
        .collect()
}

/// Copy these in (again, for one imported before: it is replaced). Its name in the library is
/// its own, with a number when another original took it first.
pub fn import(kind: Kind, paths: &[String]) -> Result<Vec<String>, String> {
    let mut meta = load();
    std::fs::create_dir_all(dir(kind)).map_err(|e| e.to_string())?;
    let mut done = vec![];
    for p in paths {
        let src = Path::new(p);
        if !src.exists() {
            return Err(format!("{p} not found"));
        }
        let lib = entries(&mut meta, kind);
        let name = match lib.iter().find(|(_, e)| e.from == *p) {
            Some((n, _)) => n.clone(),
            None => {
                let base = match kind {
                    Kind::Skill => skill_name(src),
                    Kind::Rule => src.file_stem().unwrap_or_default().to_string_lossy().to_string(),
                };
                let base: String = base.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
                let mut name = base.clone();
                let mut n = 2;
                while lib.contains_key(&name) {
                    name = format!("{base}-{n}");
                    n += 1;
                }
                name
            }
        };
        let to = path_of(kind, &name);
        if to.is_dir() {
            let _ = std::fs::remove_dir_all(&to);
        }
        copy(src, &to).map_err(|e| e.to_string())?;
        lib.insert(name.clone(), Entry { from: p.clone(), at: crate::workspace::now_ms(), hash: hash(src) });
        done.push(name);
    }
    save(&meta)?;
    Ok(done)
}

pub fn remove(kind: Kind, name: &str) -> Result<(), String> {
    let mut meta = load();
    if entries(&mut meta, kind).remove(name).is_none() {
        return Err(format!("{name} not found"));
    }
    let p = path_of(kind, name);
    let _ = if p.is_dir() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
    save(&meta)
}

/// The text of one (a Skill's SKILL.md), for its preview; `found`: a path found on this computer.
pub fn read(kind: Kind, name: Option<&str>, found: Option<&str>) -> Option<String> {
    let p = match (name, found) {
        (Some(n), _) if !n.contains('/') && !n.contains("..") => match kind {
            Kind::Skill => path_of(kind, n).join("SKILL.md"),
            Kind::Rule => path_of(kind, n),
        },
        (_, Some(f)) => {
            // Only what a scan lists.
            let p = PathBuf::from(f);
            let ok = match kind {
                Kind::Skill => p.join("SKILL.md").is_file(),
                Kind::Rule => p.extension().is_some_and(|x| x == "md" || x == "mdc"),
            };
            if !ok {
                return None;
            }
            if kind == Kind::Skill { p.join("SKILL.md") } else { p }
        }
        _ => return None,
    };
    std::fs::read_to_string(p).ok().map(|t| t.chars().take(64 * 1024).collect())
}

/// The picked rules as an agent is told them: each with its file patterns, when it has some.
pub fn rules_brief(picks: &[String]) -> String {
    let mut out = String::new();
    for n in picks {
        let Ok(text) = std::fs::read_to_string(path_of(Kind::Rule, n)) else { continue };
        let f = front(&text);
        if f.body.trim().is_empty() {
            continue;
        }
        if out.is_empty() {
            out.push_str("[Recursion] Rules the human picked for you (follow them; one that names files applies only when you work on those):\n");
        }
        let when = if f.paths.is_empty() { String::new() } else { format!(" (only when working on: {})", f.paths.join(", ")) };
        out.push_str(&format!("--- rule {n}{when} ---\n{}\n", f.body.trim_end()));
    }
    out
}

/// A folder of the picked Skills for one agent, laid out as a Claude plugin (its `skills/` is
/// also what Codex is pointed at): links into the library, so nothing is copied twice.
pub fn agent_skills_dir(key: &str, picks: &[String]) -> Result<PathBuf, String> {
    let d = root().join("run").join(format!("skills-{key}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join(".claude-plugin")).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(d.join("skills")).map_err(|e| e.to_string())?;
    std::fs::write(d.join(".claude-plugin/plugin.json"), r#"{"name":"recursion","version":"1.0.0","description":"The Skills Recursion gives this agent"}"#).map_err(|e| e.to_string())?;
    for n in picks {
        let src = path_of(Kind::Skill, n);
        if src.join("SKILL.md").is_file() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(&src, d.join("skills").join(n)).map_err(|e| e.to_string())?;
            #[cfg(not(unix))]
            copy(&src, &d.join("skills").join(n)).map_err(|e| e.to_string())?;
        }
    }
    Ok(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_frontmatter() {
        let f = front("---\nname: zebra\ndescription: >\n  Use when\n  asked\npaths:\n  - \"src/**/*.ts\"\n  - lib/*.ts\n---\nBody here\n");
        assert_eq!(f.name.as_deref(), Some("zebra"));
        assert_eq!(f.description, "Use when asked");
        assert_eq!(f.paths, vec!["src/**/*.ts", "lib/*.ts"]);
        assert_eq!(f.body.trim(), "Body here");
        let g = front("---\nglobs: src/*.ts, test/*.ts\n---\nx");
        assert_eq!(g.paths, vec!["src/*.ts", "test/*.ts"]);
        assert_eq!(front("no front").body, "no front");
    }
}
