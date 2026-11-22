//! Project files for the chat box's 📎: the list to pick from, and a file's content for the
//! viewer panel. Only files inside the session's folder; paths are relative to it.

use serde::Serialize;
use std::path::{Component, Path, PathBuf};

const MAX_FILES: usize = 20_000;
const MAX_TEXT: u64 = 2 * 1024 * 1024;
const MAX_IMAGE: u64 = 8 * 1024 * 1024;
/// Folders a walk skips when the project isn't a git repository.
const SKIP: &[&str] = &[".git", "node_modules", "target", "dist", "build", ".venv", "venv", "__pycache__", ".next", ".cache"];

/// The project's files, relative paths, sorted. A git repository lists what git tracks or would
/// (ignored files left out); any other folder is walked, skipping the usual build folders.
pub async fn list(cwd: &str) -> Vec<String> {
    let git = tokio::process::Command::new("git").args(["-C", cwd, "ls-files", "-co", "--exclude-standard", "-z"]).output().await;
    if let Ok(out) = git {
        if out.status.success() {
            let mut v: Vec<String> = out.stdout.split(|b| *b == 0).filter(|s| !s.is_empty()).map(|s| String::from_utf8_lossy(s).to_string()).take(MAX_FILES).collect();
            v.sort();
            return v;
        }
    }
    let root = PathBuf::from(cwd);
    tokio::task::spawn_blocking(move || {
        let mut out = Vec::new();
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                let Ok(kind) = e.file_type() else { continue };
                if kind.is_dir() {
                    if !name.starts_with('.') && !SKIP.contains(&name.as_str()) {
                        stack.push(e.path());
                    }
                } else if kind.is_file() {
                    if let Ok(rel) = e.path().strip_prefix(&root) {
                        out.push(rel.to_string_lossy().to_string());
                    }
                }
                if out.len() >= MAX_FILES {
                    return out;
                }
            }
        }
        out.sort();
        out
    })
    .await
    .unwrap_or_default()
}

/// A file inside `cwd`, or None when `rel` would leave it (absolute, `..`, or a link out).
pub fn resolve(cwd: &str, rel: &str) -> Option<PathBuf> {
    let p = Path::new(rel);
    if p.is_absolute() || p.components().any(|c| matches!(c, Component::ParentDir)) {
        return None;
    }
    let root = std::fs::canonicalize(cwd).ok()?;
    let full = std::fs::canonicalize(root.join(p)).ok()?;
    full.starts_with(&root).then_some(full)
}

#[derive(Serialize, Default)]
pub struct Content {
    /// The text, when it is a text file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// A data: URL, when it is an image.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub size: u64,
}

fn image_type(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "svg" => Some("image/svg+xml"),
        "ico" => Some("image/x-icon"),
        _ => None,
    }
}

/// What the viewer shows for a file: its text, an image, or why neither. `path` is one
/// `resolve` (or a grant) allowed; None says it isn't.
pub fn read(path: Option<PathBuf>) -> Content {
    let Some(path) = path else {
        return Content { error: Some(crate::i18n::t("file.err.outside")), ..Default::default() };
    };
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    if let Some(mime) = image_type(&path) {
        if size > MAX_IMAGE {
            return Content { error: Some(crate::i18n::t("file.err.tooBig")), size, ..Default::default() };
        }
        use base64::Engine;
        return match std::fs::read(&path) {
            Ok(b) => Content { image: Some(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(b))), size, ..Default::default() },
            Err(e) => Content { error: Some(e.to_string()), size, ..Default::default() },
        };
    }
    if size > MAX_TEXT {
        return Content { error: Some(crate::i18n::t("file.err.tooBig")), size, ..Default::default() };
    }
    match std::fs::read(&path) {
        // A NUL early on means it isn't text.
        Ok(b) if b.iter().take(8192).any(|&c| c == 0) => Content { error: Some(crate::i18n::t("file.err.binary")), size, ..Default::default() },
        Ok(b) => Content { text: Some(String::from_utf8_lossy(&b).to_string()), size, ..Default::default() },
        Err(e) => Content { error: Some(e.to_string()), size, ..Default::default() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stays_inside_the_project() {
        let dir = std::env::temp_dir().join(format!("rc-files-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/a.rs"), "fn a() {}").unwrap();
        let cwd = dir.to_string_lossy().to_string();
        assert!(resolve(&cwd, "src/a.rs").is_some());
        assert!(resolve(&cwd, "../etc/passwd").is_none());
        assert!(resolve(&cwd, "/etc/passwd").is_none());
        assert!(resolve(&cwd, "src/missing.rs").is_none());
        assert_eq!(read(resolve(&cwd, "src/a.rs")).text.as_deref(), Some("fn a() {}"));
        assert!(read(resolve(&cwd, "../x")).error.is_some());
        std::fs::remove_dir_all(dir).ok();
    }
}
