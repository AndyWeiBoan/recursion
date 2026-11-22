//! Sequence diagrams: participants across the top, messages down the page in order.

use super::grid::{text_block, FONT};
use super::text::{esc, width, wrap};
use super::{Issue, Out, SeqKind, Sequence};
use std::collections::HashMap;

const MARGIN: f64 = 28.0;
const SUB: f64 = 11.0;
const MSG: f64 = 12.0;
const STEP: f64 = 42.0;

pub fn render(s: &Sequence, title: bool, issues: &mut Vec<Issue>) -> Option<Out> {
    let mut ids: HashMap<&str, usize> = HashMap::new();
    for (i, p) in s.participants.iter().enumerate() {
        if ids.insert(&p.id, i).is_some() {
            issues.push(Issue::error(format!("duplicate participant `{}`", p.id)));
        }
    }
    for m in &s.messages {
        for end in [&m.from, &m.to] {
            if !ids.contains_key(end.as_str()) {
                issues.push(Issue::error(format!("message \"{}\": no participant `{end}`", m.label)));
            }
        }
    }
    if s.participants.is_empty() {
        issues.push(Issue::error("the diagram has no participants".into()));
    }
    if issues.iter().any(|i| i.error) {
        return None;
    }

    // Participant boxes.
    let heads: Vec<(Vec<String>, Vec<String>, f64, f64)> = s
        .participants
        .iter()
        .map(|p| {
            let lines = wrap(&p.label, FONT, 170.0);
            let sub = if p.sublabel.is_empty() { vec![] } else { wrap(&p.sublabel, SUB, 170.0) };
            let w = lines.iter().map(|l| width(l, FONT)).chain(sub.iter().map(|l| width(l, SUB))).fold(0.0, f64::max) + 32.0;
            let h = lines.len() as f64 * 17.0 + sub.len() as f64 * 15.0 + 22.0;
            (lines, sub, w.max(110.0), h.max(46.0))
        })
        .collect();
    let head_h = heads.iter().map(|h| h.3).fold(0.0, f64::max);
    let n = heads.len();
    // Centres: neighbours far enough apart for their boxes, then for every message label.
    let mut cx = vec![0.0f64; n];
    cx[0] = MARGIN + heads[0].2 / 2.0;
    for i in 1..n {
        cx[i] = cx[i - 1] + (heads[i - 1].2 + heads[i].2) / 2.0 + 48.0;
    }
    let msg_lines: Vec<Vec<String>> = s.messages.iter().map(|m| wrap(&m.label, MSG, 240.0)).collect();
    let label_w = |i: usize| msg_lines[i].iter().map(|l| width(l, MSG)).fold(0.0, f64::max);
    for (k, m) in s.messages.iter().enumerate() {
        let (a, b) = (ids[m.from.as_str()], ids[m.to.as_str()]);
        let (lo, hi) = (a.min(b), a.max(b));
        let need = if lo == hi { 0.0 } else { label_w(k) + 40.0 };
        if lo != hi && cx[hi] - cx[lo] < need {
            let d = need - (cx[hi] - cx[lo]);
            for x in cx.iter_mut().skip(hi) {
                *x += d;
            }
        }
        // A message to itself loops out to the right, label beside the loop.
        if lo == hi && lo + 1 < n {
            let need = 44.0 + label_w(k) + 20.0;
            if cx[lo + 1] - cx[lo] < need {
                let d = need - (cx[lo + 1] - cx[lo]);
                for x in cx.iter_mut().skip(lo + 1) {
                    *x += d;
                }
            }
        }
    }
    let title_h = if title { 40.0 } else { 0.0 };
    let top = MARGIN + title_h;
    let mut y = top + head_h + 30.0;
    let mut body = String::new();
    let mut arrows = String::new();
    for (k, m) in s.messages.iter().enumerate() {
        let (a, b) = (ids[m.from.as_str()], ids[m.to.as_str()]);
        let lines = &msg_lines[k];
        let extra = (lines.len() as f64 - 1.0) * 14.0;
        y += extra;
        let cls = match m.kind {
            SeqKind::Sync => "",
            SeqKind::Async => " async",
            SeqKind::Reply => " dashed async",
        };
        let marker = if m.kind == SeqKind::Sync { "dg-arrow" } else { "dg-open" };
        let num = if s.autonumber { format!("{}. ", k + 1) } else { String::new() };
        let mut lines = lines.clone();
        if let Some(first) = lines.first_mut() {
            *first = format!("{num}{first}");
        }
        if a == b {
            let x = cx[a];
            let d = format!("M{x:.1},{y:.1} L{:.1},{y:.1} L{:.1},{:.1} L{:.1},{:.1}", x + 36.0, x + 36.0, y + 22.0, x + 4.0, y + 22.0);
            arrows += &format!(
                r#"<g class="dg-edge{cls}" data-from="{}" data-to="{}"><path d="{d}" marker-end="url(#{marker})"/></g>"#,
                esc(&m.from),
                esc(&m.to)
            );
            body += &format!(
                r#"<g class="dg-mlabel" data-from="{}" data-to="{}">{}</g>"#,
                esc(&m.from),
                esc(&m.to),
                text_block(x + 44.0, y + 4.0 - extra, &lines, "", "start", 14.0)
            );
            y += 22.0;
        } else {
            let (x0, x1) = (cx[a], cx[b]);
            let dir = if x1 > x0 { -1.0 } else { 1.0 };
            arrows += &format!(
                r#"<g class="dg-edge{cls}" data-from="{}" data-to="{}"><path d="M{x0:.1},{y:.1} L{:.1},{y:.1}" marker-end="url(#{marker})"/></g>"#,
                esc(&m.from),
                esc(&m.to),
                x1 + dir * 2.0
            );
            body += &format!(
                r#"<g class="dg-mlabel" data-from="{}" data-to="{}">{}</g>"#,
                esc(&m.from),
                esc(&m.to),
                text_block((x0 + x1) / 2.0, y - 7.0 - extra, &lines, "", "middle", 14.0)
            );
        }
        y += STEP;
    }
    let bottom = y - STEP + 34.0;
    let mut out = String::new();
    for (i, p) in s.participants.iter().enumerate() {
        out += &format!(
            r#"<g class="dg-life" data-from="{}" data-to="{}"><path d="M{:.1},{:.1} L{:.1},{bottom:.1}"/></g>"#,
            esc(&p.id),
            esc(&p.id),
            cx[i],
            top + head_h,
            cx[i]
        );
    }
    out += &arrows;
    out += &body;
    for (i, p) in s.participants.iter().enumerate() {
        let (lines, sub, w, h) = &heads[i];
        let x = cx[i] - w / 2.0;
        let yb = top + (head_h - h) / 2.0;
        let total = lines.len() as f64 * 17.0 + sub.len() as f64 * 15.0;
        let ty = yb + h / 2.0 - total / 2.0 + 13.0;
        let mut t = text_block(cx[i], ty, lines, "dg-label", "middle", 17.0);
        if !sub.is_empty() {
            t += &text_block(cx[i], ty + lines.len() as f64 * 17.0 - 1.0, sub, "dg-sub", "middle", 15.0);
        }
        out += &format!(
            r#"<g class="dg-node k-{}" data-id="{}"><rect class="dg-shape" x="{x:.1}" y="{yb:.1}" width="{w:.1}" height="{h:.1}" rx="10"/>{t}</g>"#,
            esc(&p.kind),
            esc(&p.id)
        );
    }
    let width = cx[n - 1] + heads[n - 1].2 / 2.0 + MARGIN + s.messages.iter().enumerate().filter(|(_, m)| m.from == m.to && ids[m.from.as_str()] == n - 1).map(|(k, _)| 50.0 + label_w(k)).fold(0.0, f64::max);
    Some(Out { body: out, width, height: bottom + MARGIN, title_h })
}
