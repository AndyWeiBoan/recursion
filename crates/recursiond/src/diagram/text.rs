//! Text measurement without a font file: per-character widths close enough to size boxes and
//! keep labels from colliding. CJK and other wide characters count as one em.

pub fn char_w(c: char) -> f64 {
    match c {
        _ if (c as u32) >= 0x2E80 => 1.0,
        'i' | 'l' | 'j' | '.' | ',' | ':' | ';' | '\'' | '|' | '!' | 'I' => 0.3,
        ' ' => 0.3,
        'f' | 't' | 'r' | '(' | ')' | '[' | ']' | '/' | '-' => 0.4,
        'm' | 'w' | 'M' | 'W' => 0.85,
        'A'..='Z' => 0.66,
        '0'..='9' => 0.58,
        _ => 0.56,
    }
}

pub fn width(s: &str, size: f64) -> f64 {
    s.chars().map(char_w).sum::<f64>() * size
}

/// Greedy wrap to `max` px: Latin text breaks at spaces, CJK between any two characters. A word
/// longer than a line is split where it must.
pub fn wrap(s: &str, size: f64, max: f64) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    // Tokens: runs of non-space Latin, single wide characters, and spaces.
    let mut tokens: Vec<String> = Vec::new();
    for c in s.chars() {
        let wide = (c as u32) >= 0x2E80;
        match tokens.last_mut() {
            Some(t) if !wide && c != ' ' && !t.ends_with(' ') && !t.chars().last().is_some_and(|l| (l as u32) >= 0x2E80) => t.push(c),
            _ => tokens.push(c.to_string()),
        }
    }
    for tok in tokens {
        if width(&format!("{cur}{tok}"), size) <= max || cur.trim().is_empty() && width(&tok, size) <= max {
            cur.push_str(&tok);
            continue;
        }
        if !cur.trim().is_empty() {
            lines.push(cur.trim_end().to_string());
        }
        cur = String::new();
        if tok == " " {
            continue;
        }
        // Still too long on its own: cut it.
        for c in tok.chars() {
            if width(&format!("{cur}{c}"), size) > max && !cur.is_empty() {
                lines.push(std::mem::take(&mut cur));
            }
            cur.push(c);
        }
    }
    if !cur.trim().is_empty() {
        lines.push(cur.trim_end().to_string());
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// Escapes text for SVG content and attribute values.
pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_latin_at_spaces_and_cjk_anywhere() {
        assert_eq!(wrap("read the cache first", 13.0, 80.0), vec!["read the", "cache first"]);
        let cjk = wrap("先查快取沒有命中再查資料庫", 13.0, 60.0);
        assert!(cjk.len() > 1 && cjk.iter().all(|l| width(l, 13.0) <= 60.0), "{cjk:?}");
        assert_eq!(wrap("", 13.0, 80.0), vec![""]);
    }
}
