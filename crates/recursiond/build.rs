// Embeds every `locales/*.json` (shared with the UI) so a new language is one new file.
use std::{env, fs, path::Path};

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../locales");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut files: Vec<_> = fs::read_dir(&dir).expect("locales/").filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
    files.sort();
    let mut out = String::from("pub const LOCALES: &[(&str, &str)] = &[\n");
    for p in &files {
        println!("cargo:rerun-if-changed={}", p.display());
        let code = p.file_stem().unwrap().to_string_lossy();
        out += &format!("    ({code:?}, include_str!({:?})),\n", p.canonicalize().unwrap());
    }
    out += "];\n";
    fs::write(Path::new(&env::var("OUT_DIR").unwrap()).join("locales.rs"), out).unwrap();
}
