//! A project's own words for its agents: the prompt the human wrote in the project's settings and
//! the guidance files (AGENTS.md, CLAUDE.md, …) they ticked among those found in its folder.
//! Recursion hands these to every agent itself, Claude's and Codex's alike, and keeps the CLIs
//! from reading such files on their own (see `CLAUDE_DISABLE` and `CODEX_NO_DOCS`), so a file
//! counts only when ticked, and only once. The human's own files for every project
//! (`~/.claude/CLAUDE.md`, `~/.codex/AGENTS.md`) are listed too, since that switch turns them off.

use serde::Serialize;
use std::path::{Path, PathBuf};

/// Set on Claude Code: it reads no CLAUDE.md at all (project or the human's own).
pub const CLAUDE_DISABLE: (&str, &str) = ("CLAUDE_CODE_DISABLE_CLAUDE_MDS", "1");
/// Given to Codex as `-c`: it reads no AGENTS.md.
pub const CODEX_NO_DOCS: &str = "project_doc_max_bytes=0";

/// The file names that are guidance for agents.
const NAMES: &[&str] = &["AGENTS.md", "AGENTS.override.md", "CLAUDE.md", "CLAUDE.local.md"];
/// Not looked into: dependencies, build output, version control.
const SKIP: &[&str] = &["node_modules", "target", "dist", "build", "vendor", "venv", ".venv", "__pycache__"];
const MAX_DEPTH: usize = 5;
const MAX_FOUND: usize = 200;
/// More than this of one file is cut (a guidance file that long is a mistake, not guidance).
const MAX_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct Found {
    /// Relative to the project folder, or `~/…` for the human's own.
    pub path: String,
    pub lines: usize,
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_default()
}

/// The human's own guidance files, for every project.
fn globals() -> Vec<(String, PathBuf)> {
    let h = home();
    let claude = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|| h.join(".claude"));
    let codex = std::env::var_os("CODEX_HOME").map(PathBuf::from).unwrap_or_else(|| h.join(".codex"));
    [("~/.claude/CLAUDE.md", claude.join("CLAUDE.md")), ("~/.codex/AGENTS.md", codex.join("AGENTS.md"))]
        .into_iter()
        .filter(|(_, p)| p.is_file())
        .map(|(n, p)| (n.to_string(), p))
        .collect()
}

fn lines(p: &Path) -> usize {
    std::fs::read_to_string(p).map(|s| s.lines().count()).unwrap_or(0)
}

/// Every guidance file in the project folder (and the human's own), the folder's own first.
pub fn scan(cwd: &str) -> Vec<Found> {
    let root = Path::new(cwd);
    let mut out = vec![];
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        let mut subdirs = vec![];
        for e in entries {
            let name = e.file_name().to_string_lossy().to_string();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                // Hidden folders hold no guidance, except GitHub's.
                if depth < MAX_DEPTH && !SKIP.contains(&name.as_str()) && (!name.starts_with('.') || name == ".github") {
                    subdirs.push(e.path());
                }
            } else if NAMES.contains(&name.as_str()) || (name == "copilot-instructions.md" && dir.file_name().is_some_and(|d| d == ".github")) {
                let rel = e.path().strip_prefix(root).map(|p| p.to_string_lossy().to_string()).unwrap_or(name);
                out.push(Found { lines: lines(&e.path()), path: rel });
            }
        }
        // Depth first, in name order: push in reverse so the first name is looked at first.
        for d in subdirs.into_iter().rev() {
            stack.push((d, depth + 1));
        }
        if out.len() >= MAX_FOUND {
            break;
        }
    }
    // The folder's own (top level) first, then deeper ones.
    out.sort_by_key(|f| (f.path.matches('/').count(), f.path.clone()));
    out.extend(globals().into_iter().map(|(n, p)| Found { lines: lines(&p), path: n }));
    out
}

/// What is ticked when the human hasn't chosen: the folder's own AGENTS.md and CLAUDE.md, and
/// the human's own files, as the CLIs would have read them.
pub fn default_picks(found: &[Found]) -> Vec<String> {
    found.iter().filter(|f| matches!(f.path.as_str(), "AGENTS.md" | "CLAUDE.md") || f.path.starts_with("~/")).map(|f| f.path.clone()).collect()
}

/// One listed file's text (only a file `scan` lists, so nothing outside the folder is read).
pub fn read(cwd: &str, path: &str) -> Option<String> {
    let full = if let Some((_, p)) = globals().into_iter().find(|(n, _)| n == path) {
        p
    } else {
        if path.split('/').any(|c| c == ".." || c.is_empty()) || Path::new(path).is_absolute() {
            return None;
        }
        let name = Path::new(path).file_name()?.to_string_lossy().to_string();
        if !NAMES.contains(&name.as_str()) && !path.ends_with(".github/copilot-instructions.md") {
            return None;
        }
        Path::new(cwd).join(path)
    };
    let mut text = std::fs::read_to_string(full).ok()?;
    if text.len() > MAX_BYTES {
        let cut = (0..=MAX_BYTES).rev().find(|&i| text.is_char_boundary(i)).unwrap_or(0);
        text.truncate(cut);
        text.push_str("\n…(cut: the file is longer)");
    }
    Some(text)
}

/// What an agent is told about its project: the ticked files, then the prompt (which the chat's
/// own rules, told after it, take precedence over). Empty when there is nothing.
pub fn brief(cwd: &str, prompt: &str, picks: &[String]) -> String {
    let mut out = String::new();
    let files: Vec<(String, String)> = picks.iter().filter_map(|p| read(cwd, p).map(|t| (p.clone(), t))).filter(|(_, t)| !t.trim().is_empty()).collect();
    if !files.is_empty() {
        out.push_str("[Recursion] Instructions files of this project that the human chose for you (follow them as the project's instructions):\n");
        for (p, t) in files {
            out.push_str(&format!("--- {p} ---\n{}\n", t.trim_end()));
        }
    }
    let prompt = prompt.trim();
    if !prompt.is_empty() {
        out.push_str(&format!("[Recursion] This project's prompt, set by the human for every chat of the project (above the instructions files; a chat's own rules go above it):\n{prompt}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("rec-guides-{}", crate::model::new_id()));
        std::fs::create_dir_all(d.join("docs")).unwrap();
        std::fs::create_dir_all(d.join("node_modules/x")).unwrap();
        std::fs::create_dir_all(d.join(".github")).unwrap();
        std::fs::write(d.join("AGENTS.md"), "use pnpm\n").unwrap();
        std::fs::write(d.join("docs/CLAUDE.md"), "docs rule\nsecond\n").unwrap();
        std::fs::write(d.join("node_modules/x/AGENTS.md"), "not this").unwrap();
        std::fs::write(d.join(".github/copilot-instructions.md"), "copilot").unwrap();
        std::fs::write(d.join("README.md"), "not guidance").unwrap();
        d
    }

    #[test]
    fn finds_guidance_files_but_not_dependencies() {
        let d = tmp();
        let found: Vec<String> = scan(d.to_str().unwrap()).into_iter().map(|f| f.path).filter(|p| !p.starts_with("~/")).collect();
        assert_eq!(found, vec!["AGENTS.md", ".github/copilot-instructions.md", "docs/CLAUDE.md"]);
        let f = scan(d.to_str().unwrap());
        assert_eq!(f.iter().find(|x| x.path == "docs/CLAUDE.md").unwrap().lines, 2);
        assert!(default_picks(&f).contains(&"AGENTS.md".to_string()));
        assert!(!default_picks(&f).contains(&"docs/CLAUDE.md".to_string()));
    }

    #[test]
    fn reads_only_guidance_inside_the_folder() {
        let d = tmp();
        let cwd = d.to_str().unwrap();
        assert_eq!(read(cwd, "AGENTS.md").as_deref(), Some("use pnpm\n"));
        assert!(read(cwd, "README.md").is_none());
        assert!(read(cwd, "../AGENTS.md").is_none());
        assert!(read(cwd, "/etc/passwd").is_none());
    }

    #[test]
    fn brief_puts_files_then_the_prompt() {
        let d = tmp();
        let b = brief(d.to_str().unwrap(), "  answer in English ", &["AGENTS.md".into(), "missing/AGENTS.md".into()]);
        assert!(b.contains("--- AGENTS.md ---\nuse pnpm"));
        assert!(b.find("AGENTS.md ---").unwrap() < b.find("answer in English").unwrap());
        assert_eq!(brief(d.to_str().unwrap(), " ", &[]), "");
    }
}
