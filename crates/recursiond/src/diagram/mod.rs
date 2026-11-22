//! Diagrams agents describe as JSON and we draw as SVG: architecture, sequence and workflow.
//! The idea and the shape of the input follow Archify (MIT, github.com/tt-a1i/archify); the code
//! is our own. Boxes go in grid cells the agent picks; we size, place and route everything, and
//! report what doesn't fit so the agent can fix its input.

mod grid;
mod seq;
mod text;

use grid::{GEdge, GGroup, GNode, Grid, Shape};
use serde::Deserialize;
use serde_json::Value;
use text::esc;

#[derive(Debug, Clone)]
pub struct Issue {
    pub error: bool,
    pub msg: String,
}

impl Issue {
    fn error(msg: String) -> Self {
        Issue { error: true, msg }
    }
    fn warning(msg: String) -> Self {
        Issue { error: false, msg }
    }
}

/// A renderer's result before it is wrapped in the SVG frame.
pub(crate) struct Out {
    body: String,
    width: f64,
    height: f64,
    title_h: f64,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Spec {
    Architecture {
        #[serde(default)]
        title: String,
        components: Vec<Component>,
        #[serde(default)]
        connections: Vec<Connection>,
        #[serde(default)]
        boundaries: Vec<Boundary>,
    },
    Workflow {
        #[serde(default)]
        title: String,
        #[serde(default)]
        lanes: Vec<String>,
        nodes: Vec<Step>,
        #[serde(default)]
        edges: Vec<Link>,
    },
    Sequence {
        #[serde(default)]
        title: String,
        #[serde(flatten)]
        seq: Sequence,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Component {
    id: String,
    label: String,
    #[serde(default)]
    sublabel: String,
    #[serde(default = "backend")]
    kind: CompKind,
    row: u32,
    col: u32,
}

fn backend() -> CompKind {
    CompKind::Backend
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum CompKind {
    User,
    Frontend,
    Backend,
    Database,
    Cache,
    Queue,
    Cloud,
    Security,
    External,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Connection {
    from: String,
    to: String,
    #[serde(default)]
    label: String,
    #[serde(default, rename = "async")]
    is_async: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Boundary {
    label: String,
    members: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    id: String,
    label: String,
    #[serde(default)]
    sublabel: String,
    #[serde(default = "step")]
    kind: StepKind,
    /// A lane name (when `lanes` is given) …
    #[serde(default)]
    lane: Option<String>,
    /// … or a row number.
    #[serde(default)]
    row: Option<u32>,
    col: u32,
}

fn step() -> StepKind {
    StepKind::Step
}

#[derive(Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
enum StepKind {
    Start,
    End,
    Step,
    Decision,
    Wait,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Link {
    from: String,
    to: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    dashed: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Sequence {
    participants: Vec<Participant>,
    messages: Vec<Message>,
    #[serde(default)]
    autonumber: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Participant {
    id: String,
    label: String,
    #[serde(default)]
    sublabel: String,
    #[serde(default = "participant_kind")]
    kind: String,
}

fn participant_kind() -> String {
    "backend".into()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Message {
    from: String,
    to: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    kind: SeqKind,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SeqKind {
    #[default]
    Sync,
    Async,
    Reply,
}

fn comp_kind(k: CompKind) -> (&'static str, &'static str) {
    match k {
        CompKind::User => ("user", "☺\u{FE0E}"),
        CompKind::Frontend => ("frontend", "▭\u{FE0E}"),
        CompKind::Backend => ("backend", "⚙\u{FE0E}"),
        CompKind::Database => ("database", "⛁\u{FE0E}"),
        CompKind::Cache => ("cache", "⚡\u{FE0E}"),
        CompKind::Queue => ("queue", "⇶\u{FE0E}"),
        CompKind::Cloud => ("cloud", "☁\u{FE0E}"),
        CompKind::Security => ("security", "⛨\u{FE0E}"),
        CompKind::External => ("external", "↗\u{FE0E}"),
    }
}

/// What a diagram came out as: the SVG, plus problems worth telling the agent about.
pub struct Rendered {
    pub svg: String,
    pub warnings: Vec<String>,
}

/// Draws a diagram, or says why it can't (`Err` lists every problem found).
pub fn render(spec: &Value) -> Result<Rendered, Vec<String>> {
    let spec: Spec = serde_json::from_value(spec.clone()).map_err(|e| vec![format!("invalid diagram: {e}")])?;
    let mut issues = Vec::new();
    let (title, out) = match spec {
        Spec::Architecture { title, components, connections, boundaries } => {
            let g = Grid {
                title: title.clone(),
                nodes: components
                    .into_iter()
                    .map(|c| {
                        let (kind, glyph) = comp_kind(c.kind);
                        GNode { id: c.id, label: c.label, sub: c.sublabel, kind: kind.into(), glyph, shape: Shape::Box, row: c.row, col: c.col, dashed: matches!(c.kind, CompKind::External) }
                    })
                    .collect(),
                edges: connections.into_iter().enumerate().map(|(i, c)| GEdge { id: format!("e{i}"), from: c.from, to: c.to, label: c.label, dashed: c.is_async }).collect(),
                groups: boundaries.into_iter().map(|b| GGroup { label: b.label, members: b.members }).collect(),
                lanes: vec![],
            };
            (title, grid::render(&g, &mut issues))
        }
        Spec::Workflow { title, lanes, nodes, edges } => {
            let mut steps = Vec::new();
            for n in nodes {
                let row = match (&n.lane, n.row) {
                    (Some(l), _) => match lanes.iter().position(|x| x == l) {
                        Some(r) => r as u32,
                        None => {
                            issues.push(Issue::error(format!("node `{}`: no lane \"{l}\" (lanes are {:?})", n.id, lanes)));
                            0
                        }
                    },
                    (None, Some(r)) => r,
                    (None, None) if lanes.is_empty() => 0,
                    (None, None) => {
                        issues.push(Issue::error(format!("node `{}` needs a `lane`", n.id)));
                        0
                    }
                };
                let (kind, shape) = match n.kind {
                    StepKind::Start => ("start", Shape::Pill),
                    StepKind::End => ("end", Shape::Pill),
                    StepKind::Step => ("step", Shape::Box),
                    StepKind::Decision => ("decision", Shape::Diamond),
                    StepKind::Wait => ("wait", Shape::Box),
                };
                steps.push(GNode { id: n.id, label: n.label, sub: n.sublabel, kind: kind.into(), glyph: "", shape, row, col: n.col, dashed: n.kind == StepKind::Wait });
            }
            let g = Grid {
                title: title.clone(),
                nodes: steps,
                edges: edges.into_iter().enumerate().map(|(i, e)| GEdge { id: format!("e{i}"), from: e.from, to: e.to, label: e.label, dashed: e.dashed }).collect(),
                groups: vec![],
                lanes,
            };
            let out = if issues.iter().any(|i| i.error) { None } else { grid::render(&g, &mut issues) };
            (title, out)
        }
        Spec::Sequence { title, seq } => {
            let out = seq::render(&seq, !title.is_empty(), &mut issues);
            (title, out)
        }
    };
    let Some(out) = out else {
        return Err(issues.into_iter().filter(|i| i.error).map(|i| i.msg).collect());
    };
    let title_svg = if title.is_empty() || out.title_h == 0.0 { String::new() } else { format!(r#"<text class="dg-title" x="28" y="50">{}</text>"#, esc(&title)) };
    let (w, h) = (out.width.ceil(), out.height.ceil());
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" class="dg" viewBox="0 0 {w} {h}" width="{w}" height="{h}" font-family="ui-sans-serif, system-ui, -apple-system, 'Segoe UI', 'Noto Sans CJK TC', 'PingFang TC', sans-serif"><style>{STYLE}</style><defs><marker id="dg-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path class="dg-head" d="M0,1 L10,5 L0,9 Z"/></marker><marker id="dg-open" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="8" markerHeight="8" orient="auto-start-reverse"><path class="dg-head-open" d="M1,1 L9,5 L1,9"/></marker></defs><rect class="dg-bg" width="{w}" height="{h}"/>{title_svg}{}</svg>"#,
        out.body
    );
    Ok(Rendered { svg, warnings: issues.into_iter().map(|i| i.msg).collect() })
}

/// Colours live in the SVG itself, so an exported file looks the same; `.light` switches theme
/// and `.focus` dims everything but the highlighted (`.hl`) parts.
const STYLE: &str = "\
.dg{--bg:#1d1e22;--node:#26272c;--line:#8b8d96;--text:#e6e6e8;--dim:#9a9ca5;--lane:#ffffff07;--lane2:#ffffff0d;--group:#ffffff30;--hl:#e8714a;--lbg:#1d1e22}\
.dg.light{--bg:#fbfbfc;--node:#ffffff;--line:#7d808a;--text:#1d1e22;--dim:#6b6d76;--lane:#00000006;--lane2:#0000000c;--group:#00000030;--hl:#d45a30;--lbg:#fbfbfc}\
.dg-bg{fill:var(--bg)}\
.dg-title{fill:var(--text);font-size:17px;font-weight:600}\
.dg-shape{fill:var(--node);stroke:var(--c,var(--line));stroke-width:1.5}\
.dg-node.dashed .dg-shape{stroke-dasharray:5 4}\
.dg-accent{fill:var(--c,var(--line))}\
.dg-glyph{fill:var(--c,var(--dim));font-size:14px}\
.dg-label{fill:var(--text);font-size:13px;font-weight:600}\
.dg-sub{fill:var(--dim);font-size:11px}\
.k-user{--c:#c792ea}.k-frontend{--c:#6aa6ff}.k-backend{--c:#3fb8a8}.k-database{--c:#e0a83a}.k-cache{--c:#e8714a}.k-queue{--c:#e05d8c}.k-cloud{--c:#93c0ff}.k-security{--c:#ff6b6b}.k-external{--c:#8b8d96}\
.k-start{--c:#4ab36b}.k-end{--c:#e8714a}.k-step{--c:#6aa6ff}.k-decision{--c:#e0a83a}.k-wait{--c:#8b8d96}\
.dg-edge path{fill:none;stroke:var(--line);stroke-width:1.5}\
.dg-edge.dashed path{stroke-dasharray:6 5}\
.dg-head{fill:var(--line)}.dg-head-open{fill:none;stroke:var(--line);stroke-width:1.5}\
.dg-elabel rect{fill:var(--lbg);opacity:.92}\
.dg-elabel text,.dg-mlabel text{fill:var(--dim);font-size:11px}\
.dg-mlabel text{font-size:12px;fill:var(--text)}\
.dg-life path{stroke:var(--line);stroke-width:1;stroke-dasharray:4 4;opacity:.6}\
.dg-group rect{fill:none;stroke:var(--group);stroke-width:1.2;stroke-dasharray:6 4}\
.dg-group text{fill:var(--dim);font-size:11px;font-weight:600;letter-spacing:.04em}\
.dg-lane{fill:var(--lane)}.dg-lane.alt{fill:var(--lane2)}\
.dg-lane-label{fill:var(--dim);font-size:12px;font-weight:600}\
.dg-node,.dg-edge,.dg-elabel,.dg-mlabel,.dg-life{transition:opacity .15s}\
.dg.focus .dg-node,.dg.focus .dg-edge,.dg.focus .dg-elabel,.dg.focus .dg-mlabel,.dg.focus .dg-life{opacity:.15}\
.dg.focus .hl{opacity:1}\
.dg.focus .dg-edge.hl path{stroke:var(--hl);stroke-width:2}\
.dg-node{cursor:pointer}";

/// How to use `diagram_write`, for the tool's description.
pub const GUIDE: &str = r#"Draw a diagram: you describe boxes and lines as JSON, the app lays them out, routes every line around boxes, and shows it to the human in a panel (themes, zoom, export). Prefer it over hand-made SVG or Mermaid.

Pick a `type`:
- "architecture": services, databases, who talks to whom.
  {"type":"architecture","title":"…","components":[{"id":"web","label":"Web app","sublabel":"React","kind":"frontend","row":0,"col":0}, …],
   "connections":[{"from":"web","to":"api","label":"HTTPS"}, {"from":"api","to":"jobs","label":"enqueue","async":true}],
   "boundaries":[{"label":"VPC","members":["api","db"]}]}
  kind: user | frontend | backend | database | cache | queue | cloud | security | external.
- "workflow": steps and decisions, optionally in swimlanes.
  {"type":"workflow","title":"…","lanes":["Customer","System"],"nodes":[{"id":"s","label":"Order placed","kind":"start","lane":"Customer","col":0},
   {"id":"ok","label":"In stock?","kind":"decision","lane":"System","col":1}, …],"edges":[{"from":"s","to":"ok"},{"from":"ok","to":"x","label":"no"}]}
  kind: start | end | step | decision | wait. Without lanes, give each node a `row` instead.
- "sequence": calls between participants in time order.
  {"type":"sequence","title":"…","participants":[{"id":"b","label":"Browser"},{"id":"a","label":"API"}],
   "messages":[{"from":"b","to":"a","label":"GET /items"},{"from":"a","to":"b","label":"200 OK","kind":"reply"}],"autonumber":true}
  message kind: sync | async | reply.

Layout (architecture, workflow): `row` and `col` are grid cells starting at 0; one box per cell. Put the main path along one row or column, side branches in the next row/column, and leave a cell free between boxes when a line must pass between them. Keep labels short (a few words); `sublabel` is a second, smaller line.
Every id must be unique, and lines must name existing ids. The reply lists problems (overlapping labels, lines that can't avoid a box); fix them by moving boxes to other cells and call again with the same `id` to replace the diagram. Write labels in the human's language."#;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn draws_each_kind_without_complaints() {
        let arch = json!({"type":"architecture","title":"Cache","components":[
            {"id":"b","label":"Browser","kind":"user","row":0,"col":0},
            {"id":"api","label":"API","sublabel":"Rust","row":0,"col":1},
            {"id":"r","label":"Redis","kind":"cache","row":0,"col":2},
            {"id":"pg","label":"PostgreSQL","kind":"database","row":1,"col":2}],
            "connections":[{"from":"b","to":"api","label":"HTTPS"},{"from":"api","to":"r","label":"get"},{"from":"api","to":"pg","label":"miss: query"}],
            "boundaries":[{"label":"Backend","members":["api","r","pg"]}]});
        let flow = json!({"type":"workflow","lanes":["Customer","System"],"nodes":[
            {"id":"s","label":"下單","kind":"start","lane":"Customer","col":0},
            {"id":"c","label":"有庫存？","kind":"decision","lane":"System","col":1},
            {"id":"ship","label":"出貨","lane":"System","col":2},
            {"id":"e","label":"完成","kind":"end","lane":"Customer","col":3}],
            "edges":[{"from":"s","to":"c"},{"from":"c","to":"ship","label":"有"},{"from":"ship","to":"e"}]});
        let seq = json!({"type":"sequence","participants":[{"id":"b","label":"Browser"},{"id":"a","label":"API"},{"id":"d","label":"DB"}],
            "autonumber":true,"messages":[{"from":"b","to":"a","label":"GET /items"},{"from":"a","to":"d","label":"SELECT * FROM items WHERE owner = $1"},{"from":"d","to":"a","label":"rows","kind":"reply"},{"from":"a","to":"a","label":"cache"},{"from":"a","to":"b","label":"200","kind":"reply"}]});
        for (i, spec) in [arch, flow, seq].into_iter().enumerate() {
            let r = render(&spec).unwrap_or_else(|e| panic!("{e:?}"));
            // DG_OUT=<dir> keeps the drawings for a look.
            if let Ok(dir) = std::env::var("DG_OUT") {
                std::fs::write(format!("{dir}/{i}.svg"), &r.svg).unwrap();
            }
            assert!(r.warnings.is_empty(), "{:?}", r.warnings);
            assert!(r.svg.starts_with("<svg") && r.svg.contains("dg-node"));
        }
    }

    #[test]
    fn routes_around_a_box_in_the_way() {
        // a → c has b sitting between them in the same row.
        let spec = json!({"type":"architecture","components":[
            {"id":"a","label":"A","row":0,"col":0},{"id":"b","label":"B","row":0,"col":1},{"id":"c","label":"C","row":0,"col":2}],
            "connections":[{"from":"a","to":"c"}]});
        let r = render(&spec).unwrap();
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        let d = r.svg.split(r#"data-from="a" data-to="c"><path d=""#).nth(1).unwrap().split('"').next().unwrap();
        assert!(d.matches(" L").count() >= 3, "should detour: {d}");
    }

    #[test]
    fn reports_what_the_agent_must_fix() {
        let spec = json!({"type":"architecture","components":[
            {"id":"a","label":"A","row":0,"col":0},{"id":"a","label":"B","row":0,"col":0}],
            "connections":[{"from":"a","to":"zz"}]});
        let errs = render(&spec).err().unwrap();
        assert!(errs.iter().any(|e| e.contains("duplicate id")), "{errs:?}");
        assert!(errs.iter().any(|e| e.contains("both at row 0, col 0")), "{errs:?}");
        assert!(errs.iter().any(|e| e.contains("no node `zz`")), "{errs:?}");
        assert!(render(&json!({"type":"pie"})).is_err());
    }
}
