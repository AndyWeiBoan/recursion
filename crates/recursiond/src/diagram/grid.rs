//! Grid layout shared by architecture and workflow diagrams: boxes sit in (row, col) cells, the
//! renderer sizes rows and columns to fit them, then routes every edge as right-angled lines
//! through the gaps between cells, never through another box.

use super::text::{esc, width, wrap};
use super::{Issue, Out};
use std::collections::HashMap;

pub const FONT: f64 = 13.0;
const SUB: f64 = 11.0;
const LINE: f64 = 17.0;
const GAP_X: f64 = 84.0;
const GAP_Y: f64 = 64.0;
const MARGIN: f64 = 28.0;
/// Clearance kept between a route and any box it passes.
const CLEAR: f64 = 10.0;
const LANE_LABEL: f64 = 120.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    Box,
    Pill,
    Diamond,
}

pub struct GNode {
    pub id: String,
    pub label: String,
    pub sub: String,
    /// CSS class suffix for colour (`k-database`, …) and an optional glyph.
    pub kind: String,
    pub glyph: &'static str,
    pub shape: Shape,
    pub row: u32,
    pub col: u32,
    pub dashed: bool,
}

pub struct GEdge {
    pub id: String,
    pub from: String,
    pub to: String,
    pub label: String,
    pub dashed: bool,
}

pub struct GGroup {
    pub label: String,
    pub members: Vec<String>,
}

pub struct Grid {
    pub title: String,
    pub nodes: Vec<GNode>,
    pub edges: Vec<GEdge>,
    pub groups: Vec<GGroup>,
    /// Workflow swimlanes: one per row, labelled on the left.
    pub lanes: Vec<String>,
}

#[derive(Clone, Copy, Debug)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Rect {
    fn cx(&self) -> f64 {
        self.x + self.w / 2.0
    }
    fn cy(&self) -> f64 {
        self.y + self.h / 2.0
    }
    fn grow(&self, d: f64) -> Rect {
        Rect { x: self.x - d, y: self.y - d, w: self.w + 2.0 * d, h: self.h + 2.0 * d }
    }
    fn overlaps(&self, o: &Rect) -> bool {
        self.x < o.x + o.w && o.x < self.x + self.w && self.y < o.y + o.h && o.y < self.y + self.h
    }
    /// Does the axis-aligned segment a→b pass through this rectangle's inside?
    fn hit(&self, a: (f64, f64), b: (f64, f64)) -> bool {
        let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
        let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
        x1 > self.x && x0 < self.x + self.w && y1 > self.y && y0 < self.y + self.h
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Side {
    Top,
    Bottom,
    Left,
    Right,
}

struct Sized {
    lines: Vec<String>,
    sub: Vec<String>,
    w: f64,
    h: f64,
}

fn size(n: &GNode) -> Sized {
    let max = 200.0;
    let lines = wrap(&n.label, FONT, max);
    let sub = if n.sub.is_empty() { vec![] } else { wrap(&n.sub, SUB, max) };
    let text_w = lines.iter().map(|l| width(l, FONT)).chain(sub.iter().map(|l| width(l, SUB))).fold(0.0, f64::max);
    let glyph = if n.glyph.is_empty() { 0.0 } else { 20.0 };
    let mut w = (text_w + glyph + 32.0).max(120.0);
    let mut h = (lines.len() as f64 * LINE + sub.len() as f64 * 15.0 + 22.0).max(48.0);
    if n.shape == Shape::Diamond {
        // The text sits in the middle half of a diamond.
        w = (text_w * 1.6 + 40.0).max(130.0);
        h = (h * 1.5).max(76.0);
    }
    Sized { lines, sub, w, h }
}

/// One routed edge: its corner points and where its label goes.
struct Route {
    pts: Vec<(f64, f64)>,
    sides: (Side, Side),
}

pub fn render(g: &Grid, issues: &mut Vec<Issue>) -> Option<Out> {
    // ---------- checks that make a layout impossible ----------
    let mut ids: HashMap<&str, usize> = HashMap::new();
    let mut cells: HashMap<(u32, u32), &str> = HashMap::new();
    for (i, n) in g.nodes.iter().enumerate() {
        if n.id.trim().is_empty() {
            issues.push(Issue::error(format!("node #{} has no id", i + 1)));
        } else if ids.insert(&n.id, i).is_some() {
            issues.push(Issue::error(format!("duplicate id `{}`", n.id)));
        }
        if let Some(other) = cells.insert((n.row, n.col), &n.id) {
            issues.push(Issue::error(format!("`{}` and `{other}` are both at row {}, col {}; give one of them another cell", n.id, n.row, n.col)));
        }
    }
    for e in &g.edges {
        for end in [&e.from, &e.to] {
            if !ids.contains_key(end.as_str()) {
                issues.push(Issue::error(format!("edge `{}` → `{}`: no node `{end}`", e.from, e.to)));
            }
        }
    }
    for grp in &g.groups {
        for m in &grp.members {
            if !ids.contains_key(m.as_str()) {
                issues.push(Issue::error(format!("boundary \"{}\": no node `{m}`", grp.label)));
            }
        }
    }
    if g.nodes.is_empty() {
        issues.push(Issue::error("the diagram has no nodes".into()));
    }
    if issues.iter().any(|i| i.error) {
        return None;
    }

    // ---------- sizes, columns and rows ----------
    let sized: Vec<Sized> = g.nodes.iter().map(size).collect();
    let rows = g.nodes.iter().map(|n| n.row).max().unwrap_or(0) as usize + 1;
    let rows = rows.max(g.lanes.len());
    let cols = g.nodes.iter().map(|n| n.col).max().unwrap_or(0) as usize + 1;
    let mut col_w = vec![0.0f64; cols];
    let mut row_h = vec![0.0f64; rows];
    for (n, s) in g.nodes.iter().zip(&sized) {
        col_w[n.col as usize] = col_w[n.col as usize].max(s.w);
        row_h[n.row as usize] = row_h[n.row as usize].max(s.h);
    }
    for w in col_w.iter_mut() {
        if *w == 0.0 {
            *w = 40.0;
        }
    }
    for h in row_h.iter_mut() {
        if *h == 0.0 {
            *h = 40.0;
        }
    }
    let mut gap_x = vec![GAP_X; cols.saturating_sub(1)];
    let gap_y = vec![GAP_Y; rows.saturating_sub(1)];
    // A label between two neighbours in a row needs the gap between them.
    for e in &g.edges {
        let (a, b) = (&g.nodes[ids[e.from.as_str()]], &g.nodes[ids[e.to.as_str()]]);
        if a.row == b.row && a.col.abs_diff(b.col) == 1 && !e.label.is_empty() {
            let i = a.col.min(b.col) as usize;
            let need = wrap(&e.label, SUB, 160.0).iter().map(|l| width(l, SUB)).fold(0.0, f64::max) + 36.0;
            gap_x[i] = gap_x[i].max(need);
        }
    }
    // Room for a boundary's frame and label inside the gaps.
    let pad = if g.groups.is_empty() { 0.0 } else { 18.0 };
    let title_h = if g.title.is_empty() { 0.0 } else { 40.0 };
    let left = MARGIN + if g.lanes.is_empty() { 0.0 } else { LANE_LABEL } + pad;
    let top = MARGIN + title_h + pad + if g.groups.is_empty() { 0.0 } else { 16.0 };
    let mut col_x = vec![left; cols];
    for c in 1..cols {
        col_x[c] = col_x[c - 1] + col_w[c - 1] + gap_x[c - 1];
    }
    let mut row_y = vec![top; rows];
    for r in 1..rows {
        row_y[r] = row_y[r - 1] + row_h[r - 1] + gap_y[r - 1];
    }
    let rects: Vec<Rect> = g
        .nodes
        .iter()
        .zip(&sized)
        .map(|(n, s)| {
            let (c, r) = (n.col as usize, n.row as usize);
            Rect { x: col_x[c] + (col_w[c] - s.w) / 2.0, y: row_y[r] + (row_h[r] - s.h) / 2.0, w: s.w, h: s.h }
        })
        .collect();
    let content_w = col_x[cols - 1] + col_w[cols - 1] + pad + MARGIN;
    let content_h = row_y[rows - 1] + row_h[rows - 1] + pad + MARGIN;

    // Gutter lines a route may run along: the middle of every gap, plus outside the grid.
    let mut gut_x: Vec<f64> = (0..cols.saturating_sub(1)).map(|c| col_x[c] + col_w[c] + gap_x[c] / 2.0).collect();
    gut_x.push(left - pad - 14.0);
    gut_x.push(col_x[cols - 1] + col_w[cols - 1] + pad + 14.0);
    let mut gut_y: Vec<f64> = (0..rows.saturating_sub(1)).map(|r| row_y[r] + row_h[r] + gap_y[r] / 2.0).collect();
    gut_y.push(top - pad - 14.0);
    gut_y.push(row_y[rows - 1] + row_h[rows - 1] + pad + 14.0);

    // ---------- routes ----------
    let obstacles: Vec<Rect> = rects.iter().map(|r| r.grow(CLEAR)).collect();
    let mut routes: Vec<Option<Route>> = Vec::new();
    let mut placed: Vec<Vec<(f64, f64)>> = Vec::new();
    for e in &g.edges {
        let (ai, bi) = (ids[e.from.as_str()], ids[e.to.as_str()]);
        if ai == bi {
            issues.push(Issue::warning(format!("edge `{}` → `{}` points at itself; it is not drawn", e.from, e.to)));
            routes.push(None);
            continue;
        }
        let r = best_route(rects[ai], rects[bi], ai, bi, &obstacles, &gut_x, &gut_y, &placed);
        match r {
            Some(r) => {
                placed.push(r.pts.clone());
                routes.push(Some(r));
            }
            None => {
                issues.push(Issue::warning(format!(
                    "edge `{}` → `{}` can't reach its target without crossing another box; move one of them to a free row or column",
                    e.from, e.to
                )));
                let (a, b) = (rects[ai], rects[bi]);
                routes.push(Some(Route { pts: vec![(a.cx(), a.cy()), (b.cx(), b.cy())], sides: (Side::Right, Side::Left) }));
            }
        }
    }
    spread_ports(g, &ids, &rects, &mut routes);

    // ---------- labels ----------
    let mut label_boxes: Vec<Rect> = Vec::new();
    let mut labels: Vec<Option<(f64, f64, Vec<String>, f64, f64)>> = Vec::new();
    for (e, r) in g.edges.iter().zip(&routes) {
        let (Some(r), false) = (r, e.label.is_empty()) else {
            labels.push(None);
            continue;
        };
        let lines = wrap(&e.label, SUB, 160.0);
        let lw = lines.iter().map(|l| width(l, SUB)).fold(0.0, f64::max) + 10.0;
        let lh = lines.len() as f64 * 14.0 + 4.0;
        // Try along the longest segment first, then the others.
        let mut segs: Vec<usize> = (0..r.pts.len() - 1).collect();
        let len = |i: usize| (r.pts[i].0 - r.pts[i + 1].0).abs() + (r.pts[i].1 - r.pts[i + 1].1).abs();
        segs.sort_by(|a, b| len(*b).total_cmp(&len(*a)));
        let mut spot = None;
        'find: for s in segs {
            let (a, b) = (r.pts[s], r.pts[s + 1]);
            for t in [0.5, 0.3, 0.7] {
                let (cx, cy) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
                let bx = Rect { x: cx - lw / 2.0, y: cy - lh / 2.0, w: lw, h: lh };
                if !rects.iter().any(|n| n.grow(2.0).overlaps(&bx)) && !label_boxes.iter().any(|l| l.overlaps(&bx)) {
                    spot = Some(bx);
                    break 'find;
                }
            }
        }
        let bx = spot.unwrap_or_else(|| {
            issues.push(Issue::warning(format!("the label \"{}\" on `{}` → `{}` has no free spot; shorten it or add space around that edge", e.label, e.from, e.to)));
            let mid = r.pts.len() / 2;
            let (a, b) = (r.pts[mid - 1], r.pts[mid]);
            Rect { x: (a.0 + b.0) / 2.0 - lw / 2.0, y: (a.1 + b.1) / 2.0 - lh / 2.0, w: lw, h: lh }
        });
        label_boxes.push(bx);
        labels.push(Some((bx.x, bx.y, lines, lw, lh)));
    }

    // ---------- boundaries ----------
    let mut frames = Vec::new();
    for grp in &g.groups {
        let mem: Vec<usize> = grp.members.iter().map(|m| ids[m.as_str()]).collect();
        if mem.is_empty() {
            continue;
        }
        let x0 = mem.iter().map(|&i| rects[i].x).fold(f64::MAX, f64::min) - pad;
        let y0 = mem.iter().map(|&i| rects[i].y).fold(f64::MAX, f64::min) - pad - 14.0;
        let x1 = mem.iter().map(|&i| rects[i].x + rects[i].w).fold(f64::MIN, f64::max) + pad;
        let y1 = mem.iter().map(|&i| rects[i].y + rects[i].h).fold(f64::MIN, f64::max) + pad;
        let frame = Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
        for (i, r) in rects.iter().enumerate() {
            if !mem.contains(&i) && frame.overlaps(r) {
                issues.push(Issue::warning(format!(
                    "boundary \"{}\" covers `{}`, which isn't one of its members; move `{}` out of the members' rows and columns",
                    grp.label, g.nodes[i].id, g.nodes[i].id
                )));
            }
        }
        frames.push((frame, grp.label.clone()));
    }

    // ---------- SVG ----------
    let (w, h) = (content_w, content_h);
    let mut s = String::new();
    if !g.lanes.is_empty() {
        for (r, lane) in g.lanes.iter().enumerate() {
            let y0 = if r == 0 { top - pad - 8.0 } else { row_y[r] - gap_y[r - 1] / 2.0 };
            let y1 = if r + 1 == rows { row_y[r] + row_h[r] + pad + 8.0 } else { row_y[r] + row_h[r] + gap_y[r] / 2.0 };
            s += &format!(
                r#"<rect class="dg-lane{}" x="{MARGIN}" y="{y0:.1}" width="{:.1}" height="{:.1}" rx="8"/>"#,
                if r % 2 == 1 { " alt" } else { "" },
                w - 2.0 * MARGIN,
                y1 - y0
            );
            let lines = wrap(lane, FONT, LANE_LABEL - 24.0);
            let ly = (y0 + y1) / 2.0 - (lines.len() as f64 - 1.0) * LINE / 2.0;
            s += &text_block(MARGIN + 14.0, ly, &lines, "dg-lane-label", "start", LINE);
        }
    }
    for (f, label) in &frames {
        s += &format!(r#"<g class="dg-group"><rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" rx="10"/>"#, f.x, f.y, f.w, f.h);
        s += &format!(r#"<text x="{:.1}" y="{:.1}">{}</text></g>"#, f.x + 12.0, f.y + 16.0, esc(label));
    }
    for (e, r) in g.edges.iter().zip(&routes) {
        let Some(r) = r else { continue };
        let d: String = r.pts.iter().enumerate().map(|(i, p)| format!("{}{:.1},{:.1}", if i == 0 { "M" } else { " L" }, p.0, p.1)).collect();
        s += &format!(
            r#"<g class="dg-edge{}" data-id="{}" data-from="{}" data-to="{}"><path d="{d}" marker-end="url(#dg-arrow)"/></g>"#,
            if e.dashed { " dashed" } else { "" },
            esc(&e.id),
            esc(&e.from),
            esc(&e.to)
        );
    }
    for ((e, l), _) in g.edges.iter().zip(&labels).zip(&routes) {
        let Some((x, y, lines, lw, lh)) = l else { continue };
        s += &format!(
            r#"<g class="dg-elabel" data-from="{}" data-to="{}"><rect x="{x:.1}" y="{y:.1}" width="{lw:.1}" height="{lh:.1}" rx="4"/>{}</g>"#,
            esc(&e.from),
            esc(&e.to),
            text_block(x + lw / 2.0, y + 13.0, lines, "", "middle", 14.0)
        );
    }
    for ((n, sz), r) in g.nodes.iter().zip(&sized).zip(&rects) {
        s += &node_svg(n, sz, r);
    }
    Some(Out { body: s, width: w, height: h, title_h })
}

fn node_svg(n: &GNode, sz: &Sized, r: &Rect) -> String {
    let shape = match n.shape {
        Shape::Box => format!(r#"<rect class="dg-shape" x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" rx="10"/>"#, r.x, r.y, r.w, r.h),
        Shape::Pill => format!(r#"<rect class="dg-shape" x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" rx="{:.1}"/>"#, r.x, r.y, r.w, r.h, r.h / 2.0),
        Shape::Diamond => format!(
            r#"<path class="dg-shape" d="M{:.1},{:.1} L{:.1},{:.1} L{:.1},{:.1} L{:.1},{:.1} Z"/>"#,
            r.cx(),
            r.y,
            r.x + r.w,
            r.cy(),
            r.cx(),
            r.y + r.h,
            r.x,
            r.cy()
        ),
    };
    let accent = if n.shape == Shape::Box { format!(r#"<rect class="dg-accent" x="{:.1}" y="{:.1}" width="4" height="{:.1}" rx="2"/>"#, r.x + 5.0, r.y + 10.0, r.h - 20.0) } else { String::new() };
    let glyph_w = if n.glyph.is_empty() { 0.0 } else { 20.0 };
    let total = sz.lines.len() as f64 * LINE + sz.sub.len() as f64 * 15.0;
    let cx = r.cx() + glyph_w / 2.0;
    let y = r.cy() - total / 2.0 + 13.0;
    let glyph = if n.glyph.is_empty() {
        String::new()
    } else {
        let text_w = sz.lines.iter().map(|l| width(l, FONT)).chain(sz.sub.iter().map(|l| width(l, SUB))).fold(0.0, f64::max);
        format!(r#"<text class="dg-glyph" x="{:.1}" y="{:.1}" text-anchor="middle">{}</text>"#, cx - text_w / 2.0 - 12.0, r.cy() + 5.0, n.glyph)
    };
    let mut t = text_block(cx, y, &sz.lines, "dg-label", "middle", LINE);
    if !sz.sub.is_empty() {
        t += &text_block(cx, y + sz.lines.len() as f64 * LINE - 1.0, &sz.sub, "dg-sub", "middle", 15.0);
    }
    format!(
        r#"<g class="dg-node k-{}{}" data-id="{}">{shape}{accent}{glyph}{t}</g>"#,
        esc(&n.kind),
        if n.dashed { " dashed" } else { "" },
        esc(&n.id)
    )
}

pub fn text_block(x: f64, y: f64, lines: &[String], class: &str, anchor: &str, step: f64) -> String {
    let spans: String = lines.iter().enumerate().map(|(i, l)| format!(r#"<tspan x="{x:.1}" dy="{}">{}</tspan>"#, if i == 0 { 0.0 } else { step }, esc(l))).collect();
    let class = if class.is_empty() { String::new() } else { format!(r#" class="{class}""#) };
    format!(r#"<text{class} x="{x:.1}" y="{y:.1}" text-anchor="{anchor}">{spans}</text>"#)
}

fn port(r: &Rect, side: Side) -> (f64, f64) {
    match side {
        Side::Top => (r.cx(), r.y),
        Side::Bottom => (r.cx(), r.y + r.h),
        Side::Left => (r.x, r.cy()),
        Side::Right => (r.x + r.w, r.cy()),
    }
}

/// A point just outside the port, so the first and last segments leave and enter at right angles.
fn out(p: (f64, f64), side: Side, d: f64) -> (f64, f64) {
    match side {
        Side::Top => (p.0, p.1 - d),
        Side::Bottom => (p.0, p.1 + d),
        Side::Left => (p.0 - d, p.1),
        Side::Right => (p.0 + d, p.1),
    }
}

fn simplify(pts: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    let mut v: Vec<(f64, f64)> = Vec::new();
    for p in pts {
        if v.last().is_some_and(|l| (l.0 - p.0).abs() < 0.5 && (l.1 - p.1).abs() < 0.5) {
            continue;
        }
        if v.len() >= 2 {
            let (a, b) = (v[v.len() - 2], v[v.len() - 1]);
            let collinear = ((a.0 - b.0).abs() < 0.5 && (b.0 - p.0).abs() < 0.5) || ((a.1 - b.1).abs() < 0.5 && (b.1 - p.1).abs() < 0.5);
            if collinear {
                v.pop();
            }
        }
        v.push(p);
    }
    v
}

fn crossings(pts: &[(f64, f64)], others: &[Vec<(f64, f64)>]) -> usize {
    let mut n = 0;
    for o in others {
        for a in pts.windows(2) {
            for b in o.windows(2) {
                let (ah, bh) = ((a[0].1 - a[1].1).abs() < 0.5, (b[0].1 - b[1].1).abs() < 0.5);
                if ah == bh {
                    continue;
                }
                let (h, v) = if ah { (a, b) } else { (b, a) };
                let (hx0, hx1, hy) = (h[0].0.min(h[1].0), h[0].0.max(h[1].0), h[0].1);
                let (vy0, vy1, vx) = (v[0].1.min(v[1].1), v[0].1.max(v[1].1), v[0].0);
                if vx > hx0 + 1.0 && vx < hx1 - 1.0 && hy > vy0 + 1.0 && hy < vy1 - 1.0 {
                    n += 1;
                }
            }
        }
    }
    n
}

/// Tries straight lines, one-bend and two-bend routes through every gutter, and keeps the
/// cheapest that touches no box but its own two: few bends, short, few crossings.
#[allow(clippy::too_many_arguments)]
fn best_route(a: Rect, b: Rect, ai: usize, bi: usize, obstacles: &[Rect], gut_x: &[f64], gut_y: &[f64], placed: &[Vec<(f64, f64)>]) -> Option<Route> {
    let sides = [Side::Top, Side::Bottom, Side::Left, Side::Right];
    let mut cands: Vec<(Vec<(f64, f64)>, Side, Side)> = Vec::new();
    for &sa in &sides {
        for &sb in &sides {
            let (pa, pb) = (port(&a, sa), port(&b, sb));
            let (oa, ob) = (out(pa, sa, CLEAR + 4.0), out(pb, sb, CLEAR + 4.0));
            let vert = |s: Side| matches!(s, Side::Top | Side::Bottom);
            // Direct: the two outside points line up, or one bend joins them.
            cands.push((vec![pa, oa, (oa.0, ob.1), ob, pb], sa, sb));
            cands.push((vec![pa, oa, (ob.0, oa.1), ob, pb], sa, sb));
            // Through a vertical gutter, or a horizontal one.
            for &gx in gut_x {
                cands.push((vec![pa, oa, (gx, oa.1), (gx, ob.1), ob, pb], sa, sb));
                if vert(sa) {
                    continue;
                }
            }
            for &gy in gut_y {
                cands.push((vec![pa, oa, (oa.0, gy), (ob.0, gy), ob, pb], sa, sb));
            }
        }
    }
    let mut best: Option<(f64, Route)> = None;
    for (pts, sa, sb) in cands {
        let pts = simplify(pts);
        // Every segment must be axis-aligned.
        if pts.windows(2).any(|w| (w[0].0 - w[1].0).abs() > 0.5 && (w[0].1 - w[1].1).abs() > 0.5) {
            continue;
        }
        // Leave and enter perpendicular to the side, and never run back into the own box.
        let blocked = pts.windows(2).enumerate().any(|(i, w)| {
            obstacles.iter().enumerate().any(|(k, o)| {
                if k == ai && i == 0 || k == bi && i == pts.len() - 2 {
                    return false;
                }
                o.hit(w[0], w[1])
            })
        });
        if blocked {
            continue;
        }
        let bends = pts.len() as f64 - 2.0;
        let length: f64 = pts.windows(2).map(|w| (w[0].0 - w[1].0).abs() + (w[0].1 - w[1].1).abs()).sum();
        let cost = bends * 60.0 + length + crossings(&pts, placed) as f64 * 80.0;
        if best.as_ref().is_none_or(|(c, _)| cost < *c) {
            best = Some((cost, Route { pts, sides: (sa, sb) }));
        }
    }
    best.map(|(_, r)| r)
}

/// Several routes leaving the same side of a box would overlap: fan their ports out along it.
fn spread_ports(g: &Grid, ids: &HashMap<&str, usize>, rects: &[Rect], routes: &mut [Option<Route>]) {
    let mut by_side: HashMap<(usize, Side), Vec<(usize, bool)>> = HashMap::new();
    for (i, (e, r)) in g.edges.iter().zip(routes.iter()).enumerate() {
        let Some(r) = r else { continue };
        if r.pts.len() < 2 {
            continue;
        }
        by_side.entry((ids[e.from.as_str()], r.sides.0)).or_default().push((i, true));
        by_side.entry((ids[e.to.as_str()], r.sides.1)).or_default().push((i, false));
    }
    for ((node, side), mut ends) in by_side {
        if ends.len() < 2 {
            continue;
        }
        let r = rects[node];
        let horizontal = matches!(side, Side::Top | Side::Bottom);
        // Order by where each route heads, so neighbours don't cross.
        let toward = |(i, start): &(usize, bool)| {
            let pts = &routes[*i].as_ref().unwrap().pts;
            let p = if *start { pts[pts.len().min(3) - 1] } else { pts[pts.len().saturating_sub(3)] };
            if horizontal { p.0 } else { p.1 }
        };
        ends.sort_by(|a, b| toward(a).total_cmp(&toward(b)));
        let span = if horizontal { r.w } else { r.h };
        let step = (span / (ends.len() as f64 + 1.0)).min(22.0);
        let n = ends.len() as f64;
        for (k, (i, start)) in ends.iter().enumerate() {
            let d = (k as f64 - (n - 1.0) / 2.0) * step;
            let pts = &mut routes[*i].as_mut().unwrap().pts;
            if pts.len() == 2 {
                // A straight line: make room for the shift with a jog in the middle.
                let (a, b) = (pts[0], pts[1]);
                let m = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
                *pts = if horizontal { vec![a, (a.0, m.1), (b.0, m.1), b] } else { vec![a, (m.0, a.1), (m.0, b.1), b] };
            }
            let (p0, p1) = if *start { (0, 1) } else { (pts.len() - 1, pts.len() - 2) };
            if horizontal {
                pts[p0].0 += d;
                pts[p1].0 += d;
            } else {
                pts[p0].1 += d;
                pts[p1].1 += d;
            }
        }
    }
}
