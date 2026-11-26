//! A chat's todo lists: named lists whose items the human orders by dragging, ticks off, or puts
//! away in the list's 完成區. An item being done shows who is on it; an agent takes an item with
//! `todo_claim`, which fails while someone else holds it, so two agents never do the same one.
//! The hold ends when the agent finishes, lets go, or disconnects. Agents read the lists with
//! their next turn whenever they changed. Saved with the chat.

use super::Hub;
use crate::workspace::{now_ms, Todo, TodoList, TodoState};
use serde_json::{json, Value};

/// Lists and items are named by #number, or a list by its name too.
fn find_list<'a>(lists: &'a mut [TodoList], key: &str) -> Option<&'a mut TodoList> {
    let key = key.trim();
    let n = key.trim_start_matches('#');
    lists.iter_mut().find(|l| l.id == key || l.num.to_string() == n || l.name.eq_ignore_ascii_case(key))
}

fn find_item<'a>(list: &'a mut TodoList, key: &str) -> Option<&'a mut Todo> {
    let n = key.trim().trim_start_matches('#');
    list.items.iter_mut().find(|t| t.id == key || t.num.to_string() == n)
}

fn progress(l: &TodoList) -> String {
    format!("{}/{}", l.items.iter().filter(|t| t.state == TodoState::Done).count(), l.items.len())
}

impl Hub {
    fn emit_todos(&self, panel: &str) {
        let lists = self.with_ws(panel, |ws| ws.todo_lists.clone());
        self.emit(json!({ "t": "ws_todos", "panel": panel, "lists": lists }));
        self.mark_dirty(panel);
    }

    /// Changes one list (found by id, #number or name) and tells everyone.
    fn with_list<R>(&self, panel: &str, list: &str, f: impl FnOnce(&mut TodoList) -> Result<R, String>) -> Result<R, String> {
        let r = self.with_ws(panel, |ws| match find_list(&mut ws.todo_lists, list) {
            Some(l) => f(l),
            None => Err(format!("No list `{list}`. Use todo_lists to see them.")),
        })?;
        self.emit_todos(panel);
        Ok(r)
    }

    pub fn todo_list_create(&self, panel: &str, name: &str) -> Result<TodoList, String> {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err("The list needs a name.".into());
        }
        let l = self.with_ws(panel, |ws| {
            if ws.todo_lists.iter().any(|l| l.name.eq_ignore_ascii_case(&name)) {
                return Err(format!("There is already a list named {name}."));
            }
            ws.next_list += 1;
            let l = TodoList { id: crate::model::new_id(), num: ws.next_list, name, items: vec![], next_item: 0, created: now_ms() };
            ws.todo_lists.push(l.clone());
            Ok(l)
        })?;
        self.emit_todos(panel);
        Ok(l)
    }

    pub fn todo_list_rename(&self, panel: &str, list: &str, name: &str) -> Result<(), String> {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err("The list needs a name.".into());
        }
        self.with_list(panel, list, |l| {
            l.name = name;
            Ok(())
        })
    }

    pub fn todo_list_remove(&self, panel: &str, list: &str) {
        self.with_ws(panel, |ws| ws.todo_lists.retain(|l| l.id != list));
        self.emit_todos(panel);
    }

    pub fn todo_add(&self, panel: &str, list: &str, by: &str, text: &str, note: &str) -> Result<Todo, String> {
        let text = text.trim().to_string();
        if text.is_empty() {
            return Err("The item needs some text.".into());
        }
        self.with_list(panel, list, |l| {
            l.next_item += 1;
            let t = Todo { id: crate::model::new_id(), num: l.next_item, text, note: note.trim().to_string(), state: TodoState::Todo, worker: None, by: by.to_string(), done_by: None, archived: false, created: now_ms() };
            // New items go above the 完成區.
            let at = l.items.iter().position(|x| x.archived).unwrap_or(l.items.len());
            l.items.insert(at, t.clone());
            Ok(t)
        })
    }

    /// The human's change: new wording, a new card (empty clears it), or a new state (theirs to
    /// hold while doing it).
    pub fn todo_update(&self, panel: &str, list: &str, item: &str, text: Option<String>, note: Option<String>, state: Option<TodoState>) -> Result<(), String> {
        self.with_list(panel, list, |l| {
            let t = find_item(l, item).ok_or("No such item.")?;
            if let Some(text) = text.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
                t.text = text;
            }
            if let Some(note) = note {
                t.note = note.trim().to_string();
            }
            if let Some(s) = state {
                set_state(t, s, "you");
            }
            Ok(())
        })
    }

    /// Dragged to `index` among the list's items, in or out of the 完成區.
    pub fn todo_move(&self, panel: &str, list: &str, item: &str, index: usize, archived: bool) -> Result<(), String> {
        self.with_list(panel, list, |l| {
            let from = l.items.iter().position(|t| t.id == item).ok_or("No such item.")?;
            let mut t = l.items.remove(from);
            t.archived = archived;
            // The 完成區 stays below the rest, whatever index was asked for.
            let first_archived = l.items.iter().position(|x| x.archived).unwrap_or(l.items.len());
            let at = if archived { index.clamp(first_archived, l.items.len()) } else { index.min(first_archived) };
            l.items.insert(at, t);
            Ok(())
        })
    }

    pub fn todo_remove(&self, panel: &str, list: &str, item: &str) -> Result<(), String> {
        self.with_list(panel, list, |l| {
            l.items.retain(|t| t.id != item);
            Ok(())
        })
    }

    /// An agent went away (its process ended, or it left the chat): what it was on is free again.
    pub(super) fn todo_release_all(&self, panel: &str, member: &str) {
        let freed = self.with_ws(panel, |ws| {
            let mut n = 0;
            for t in ws.todo_lists.iter_mut().flat_map(|l| l.items.iter_mut()) {
                if t.state == TodoState::Doing && t.worker.as_deref() == Some(member) {
                    t.state = TodoState::Todo;
                    t.worker = None;
                    n += 1;
                }
            }
            n
        });
        if freed > 0 {
            self.emit_todos(panel);
        }
    }

    /// The lists as agents read them: what's still open in each, and who is on what.
    pub(super) fn todo_text(&self, panel: &str) -> String {
        let lists = self.with_ws(panel, |ws| ws.todo_lists.clone());
        lists.iter().map(|l| self.list_text(panel, l, false)).collect::<Vec<_>>().join("\n")
    }

    fn list_text(&self, panel: &str, l: &TodoList, all: bool) -> String {
        let name = |m: &str| if m == "you" { "the human".to_string() } else { format!("@{}", self.alias_of(panel, m).unwrap_or_else(|| "agent".into())) };
        let mut out = format!("List #{} \"{}\" ({} done)", l.num, l.name, progress(l));
        let put_away = l.items.iter().filter(|t| t.archived).count();
        for t in l.items.iter().filter(|t| all || !t.archived) {
            let mark = match t.state {
                TodoState::Todo => "[ ]",
                TodoState::Doing => "[~]",
                TodoState::Done => "[x]",
            };
            out.push_str(&format!("\n  #{} {mark} {}", t.num, t.text));
            // The cards go with todo_read only: every turn would otherwise carry them all.
            if !t.note.is_empty() {
                if all {
                    out.push_str(&format!("\n      card:\n{}", t.note.lines().map(|l| format!("      | {l}")).collect::<Vec<_>>().join("\n")));
                } else {
                    out.push_str(" — has a card (todo_read shows it)");
                }
            }
            if let (TodoState::Doing, Some(w)) = (t.state, &t.worker) {
                out.push_str(&format!(" — {} is on it", name(w)));
            }
            if t.archived {
                out.push_str(" — put away");
            }
        }
        if !all && put_away > 0 {
            out.push_str(&format!("\n  ({put_away} put away; todo_read shows them)"));
        }
        out
    }

    /// The `todo_*` MCP tools.
    pub fn todo_tool(&self, panel: &str, member: &str, name: &str, args: &Value) -> Result<String, String> {
        let s = |k: &str| args[k].as_str().unwrap_or("").trim().to_string();
        let list = s("list");
        let result = match name {
            "todo_lists" => {
                let lists = self.with_ws(panel, |ws| ws.todo_lists.clone());
                return Ok(if lists.is_empty() {
                    "No todo lists in this chat yet. todo_add with a new list name starts one.".into()
                } else {
                    lists.iter().map(|l| format!("#{} \"{}\" ({} done)", l.num, l.name, progress(l))).collect::<Vec<_>>().join("\n")
                });
            }
            "todo_read" => {
                let l = self.with_ws(panel, |ws| find_list(&mut ws.todo_lists, &list).cloned()).ok_or_else(|| format!("No list `{list}`. Use todo_lists to see them."))?;
                return Ok(self.list_text(panel, &l, true));
            }
            "todo_add" => {
                if list.is_empty() {
                    return Err("Say which list (its name, or a new name to start one).".into());
                }
                if self.with_ws(panel, |ws| find_list(&mut ws.todo_lists, &list).is_none()) {
                    self.todo_list_create(panel, &list)?;
                }
                let texts: Vec<String> = match args.get("items").and_then(|v| v.as_array()) {
                    Some(a) => a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
                    None => vec![s("text")],
                };
                // A card only for a single item.
                let note = if texts.len() == 1 { s("note") } else { String::new() };
                let nums = texts.iter().map(|t| self.todo_add(panel, &list, member, t, &note).map(|t| format!("#{}", t.num))).collect::<Result<Vec<_>, _>>()?;
                format!("Added {} to \"{list}\".", nums.join(", "))
            }
            "todo_claim" => {
                let item = s("todo");
                let (num, holder) = self.with_list(panel, &list, |l| {
                    let t = find_item(l, &item).ok_or_else(|| format!("No item `{item}` in that list."))?;
                    match (t.state, t.worker.clone()) {
                        (TodoState::Done, _) => Err(format!("#{} is already done.", t.num)),
                        (TodoState::Doing, Some(w)) if w != member => Ok((t.num, Some(w))),
                        _ => {
                            set_state(t, TodoState::Doing, member);
                            Ok((t.num, None))
                        }
                    }
                })?;
                if let Some(w) = holder {
                    let who = if w == "you" { "the human".to_string() } else { format!("@{}", self.alias_of(panel, &w).unwrap_or_else(|| "another agent".into())) };
                    return Err(format!("#{num} is taken: {who} is on it. Pick another item."));
                }
                format!("#{num} is yours. Call todo_done when it's finished, or todo_release to let it go.")
            }
            "todo_note" => {
                let item = s("todo");
                let note = args["note"].as_str().unwrap_or("").trim().to_string();
                let num = self.with_list(panel, &list, |l| {
                    let t = find_item(l, &item).ok_or_else(|| format!("No item `{item}` in that list."))?;
                    t.note = note.clone();
                    Ok(t.num)
                })?;
                if note.is_empty() { format!("#{num}'s card cleared.") } else { format!("#{num}'s card saved.") }
            }
            "todo_done" | "todo_release" => {
                let item = s("todo");
                let done = name == "todo_done";
                let num = self.with_list(panel, &list, |l| {
                    let t = find_item(l, &item).ok_or_else(|| format!("No item `{item}` in that list."))?;
                    if let (TodoState::Doing, Some(w)) = (t.state, &t.worker) {
                        if w != member {
                            return Err(format!("#{} is someone else's: only they can finish or release it.", t.num));
                        }
                    }
                    set_state(t, if done { TodoState::Done } else { TodoState::Todo }, member);
                    Ok(t.num)
                })?;
                format!("#{num} {}.", if done { "done" } else { "released" })
            }
            _ => return Err(format!("Unknown tool {name}")),
        };
        // It made this change itself: no need to tell it the lists changed.
        let text = self.todo_text(panel);
        self.rt(panel, |r| r.todos_seen.insert(member.to_string(), text));
        Ok(result)
    }
}

fn set_state(t: &mut Todo, state: TodoState, by: &str) {
    t.state = state;
    t.worker = (state == TodoState::Doing).then(|| by.to_string());
    t.done_by = (state == TodoState::Done).then(|| by.to_string());
}

pub fn tools() -> Value {
    let list = json!({ "type": "string", "description": "The list's name (or #number)" });
    let item = json!({ "type": "string", "description": "Item #number in that list" });
    json!([
        {
            "name": "todo_lists",
            "description": "This chat's todo lists: name and how many items are done.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "todo_read",
            "description": "One todo list in full: each item's #number, [ ] todo / [~] being done (and by whom) / [x] done, its card (details) if it has one, and the ones put away.",
            "inputSchema": { "type": "object", "properties": { "list": list }, "required": ["list"] }
        },
        {
            "name": "todo_add",
            "description": "Add items to a todo list of this chat; a name no list has yet starts a new list. One task per item, one line each.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "list": list,
                    "text": { "type": "string", "description": "One item" },
                    "note": { "type": "string", "description": "Optional, with `text`: the item's card, in Markdown (details, steps, links). The human opens it by clicking the item." },
                    "items": { "type": "array", "items": { "type": "string" }, "description": "Several items at once (instead of text)" }
                },
                "required": ["list"]
            }
        },
        {
            "name": "todo_claim",
            "description": "Take an item before you work on it: it shows you are on it, and fails if someone else already is (then pick another). \
You keep it until todo_done or todo_release, or until you disconnect.",
            "inputSchema": { "type": "object", "properties": { "list": list, "todo": item }, "required": ["list", "todo"] }
        },
        {
            "name": "todo_note",
            "description": "Write an item's card: its details in Markdown (steps, links, findings). Replaces what the card held; an empty note clears it. todo_read shows the cards.",
            "inputSchema": { "type": "object", "properties": { "list": list, "todo": item, "note": { "type": "string", "description": "Markdown" } }, "required": ["list", "todo", "note"] }
        },
        {
            "name": "todo_done",
            "description": "Tick off an item you finished.",
            "inputSchema": { "type": "object", "properties": { "list": list, "todo": item }, "required": ["list", "todo"] }
        },
        {
            "name": "todo_release",
            "description": "Let go of an item you took without finishing it, so someone else can.",
            "inputSchema": { "type": "object", "properties": { "list": list, "todo": item }, "required": ["list", "todo"] }
        }
    ])
}
