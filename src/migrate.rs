//! `sop migrate`: converts a folder from the format of sopc v0.0.8 and earlier, where bases and SOPs chose
//! their agents (`agents`, `exclude`, `inherits`, `position`, `sop_order`), to the current one,
//! where each agent lists its blocks in order. It also removes fields the format no longer has
//! (`locked`, from v0.0.9 and earlier).
//!
//! The old format is read here and nowhere else. Each agent's blocks are what the old rules gave
//! it, in the order the old prompt had them, so every prompt stays the same except that the
//! agent's own text (`instructions`, now `context`) moves to the top. Files are edited as text,
//! so comments and layout survive; each edit is checked by reading the file back.

use crate::model::{self, Agent, Sop};
use crate::render::{fill_json, fill_variables, render_sop_in_prompt, render_workspace, sop_payload, sop_tools};
use crate::sopfile::{kv, lost_comments, scalar};
use crate::workspace::{load_files, load_yaml, parse_sop_file, split_front_matter, stem, Issue, Issues, CONFIG};
use regex::Regex;
use serde_json::{Map, Value as Json};
use serde_yaml_ng::{Mapping, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

static KEY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^([A-Za-z_][\w-]*|"[^"]*"|'[^']*')\s*:(?:\s|$)"#).unwrap());

/// What an agent must build after migrating: its old prompt with its own text moved to the top.
#[derive(Debug, PartialEq)]
pub struct Expected {
    pub prompt: String,
    pub tool_payload: Map<String, Json>,
    pub tools: Vec<String>,
}

#[derive(Default)]
pub struct Migration {
    /// The migrated folder's source files (relative path → text).
    pub files: BTreeMap<String, String>,
    /// One per changed file, in path order: (path, new path, what changes).
    pub changes: Vec<(String, String, String)>,
    /// Files to write (new or changed), and old files to delete afterwards.
    pub writes: Vec<(String, String)>,
    pub removes: Vec<String>,
    /// Only fields the format no longer has are removed (the folder was otherwise current).
    pub removed_fields_only: bool,
    /// Comments the edits remove: (path, "line N: # text").
    pub lost_comments: Vec<(String, Vec<String>)>,
    /// What each agent must build, by id.
    pub expected: BTreeMap<String, Expected>,
}

/// Whether loading a folder (as a file map) reports an issue with this code.
fn reports(files: &BTreeMap<String, String>, code: &str) -> bool {
    match load_files(files) {
        Err(Issues(issues)) => issues.iter().any(|i| i.code == code),
        Ok(_) => false,
    }
}

/// Whether a folder (as a file map) is in the old format.
pub fn is_old(files: &BTreeMap<String, String>) -> bool {
    reports(files, "old_format")
}

/// Whether `sop migrate` has something to do: the old format, or fields the format no longer has.
pub fn needs_migrating(files: &BTreeMap<String, String>) -> bool {
    is_old(files) || reports(files, "removed_field")
}

fn failed(path: &str, msg: impl Into<String>) -> Issue {
    Issue::error("migrate_failed", path, msg)
}

// --- the old format ----------------------------------------------------------------------------

/// A base or an SOP, as the old rules saw it.
struct OldBlock {
    id: String,
    /// None for a base.
    sop: Option<Sop>,
    text: String,
    all: bool,
    agents: Vec<String>,
    exclude: Vec<String>,
    inherits: Vec<String>,
    bottom: bool,
    /// Old locked blocks couldn't be excluded. The lock itself is dropped when migrating.
    locked: bool,
}

struct OldAgent {
    agent: Agent,
    inherits: Vec<String>,
    exclude: Vec<String>,
}

fn texts(map: &Mapping, key: &str) -> Vec<String> {
    match map.get(key) {
        Some(Value::Sequence(items)) => items.iter().map(model::scalar_text).collect(),
        Some(Value::String(s)) if s != "*" => vec![s.clone()],
        _ => vec![],
    }
}

fn flag(map: &Mapping, key: &str) -> bool {
    match map.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(v) => matches!(model::scalar_text(v).to_lowercase().as_str(), "1" | "true" | "yes" | "on" | "y" | "t"),
        None => false,
    }
}

fn old_block(id: &str, map: &Mapping, sop: Option<Sop>, text: String) -> OldBlock {
    OldBlock {
        id: id.to_string(),
        sop,
        text,
        all: map.get("agents").and_then(Value::as_str) == Some("*"),
        agents: texts(map, "agents"),
        exclude: texts(map, "exclude"),
        inherits: texts(map, "inherits"),
        bottom: map.get("position").and_then(Value::as_str) == Some("bottom"),
        locked: flag(map, "locked"),
    }
}

impl OldBlock {
    fn targets(&self, agent: &Agent) -> bool {
        let names = [agent.id.clone(), agent.platform_ref()];
        if self.exclude.iter().any(|x| names.contains(x)) {
            return false;
        }
        self.all || self.agents.iter().any(|x| names.contains(x))
    }
}

/// The blocks an agent got under the old rules, in prompt order: top bases (its `inherits`,
/// parents first, then bases that target it, by id), SOPs (`sop_order` first, then by id),
/// bottom bases.
fn old_composition<'a>(
    bases: &'a [OldBlock],
    sops: &'a [OldBlock],
    order: &[String],
    a: &OldAgent,
) -> Vec<&'a OldBlock> {
    fn visit<'a>(bases: &'a [OldBlock], id: &str, stack: &mut Vec<String>, out: &mut Vec<&'a OldBlock>) {
        if out.iter().any(|b| b.id == id) || stack.iter().any(|s| s == id) {
            return;
        }
        let Some(base) = bases.iter().find(|b| b.id == id) else { return };
        stack.push(id.to_string());
        for parent in &base.inherits {
            visit(bases, parent, stack, out);
        }
        stack.pop();
        out.push(base);
    }
    let kept = |b: &&OldBlock| b.targets(&a.agent) && (b.locked || !a.exclude.contains(&b.id));
    let mut targeted: Vec<&OldBlock> = bases.iter().filter(kept).collect();
    targeted.sort_by(|x, y| x.id.cmp(&y.id));
    let mut resolved = vec![];
    for id in &a.inherits {
        visit(bases, id, &mut vec![], &mut resolved);
    }
    for b in targeted {
        visit(bases, &b.id, &mut vec![], &mut resolved);
    }
    let mut matching: Vec<&OldBlock> = sops.iter().filter(kept).collect();
    matching.sort_by(|x, y| x.id.cmp(&y.id));
    let ordered = order.iter().filter_map(|id| matching.iter().find(|s| &s.id == id).copied());
    let rest = matching.iter().filter(|s| !order.contains(&s.id)).copied();
    let top = resolved.iter().filter(|b| !b.bottom).copied();
    let bottom = resolved.iter().filter(|b| b.bottom).copied();
    top.chain(ordered).chain(rest).chain(bottom).collect()
}

/// The old prompt, built by the old rules, with the agent's own text moved to the top.
fn expected(blocks: &[&OldBlock], agent: &Agent, config: &model::Config) -> Expected {
    let mut sections = vec![agent.context.trim().to_string()];
    sections.extend(blocks.iter().filter(|b| b.sop.is_none() && !b.bottom).map(|b| b.text.clone()));
    let sops: Vec<&Sop> = blocks.iter().filter_map(|b| b.sop.as_ref()).collect();
    if !sops.is_empty() {
        let parts: Vec<String> = sops.iter().map(|s| render_sop_in_prompt(s)).collect();
        sections.push(format!("{}\n\n{}", config.sops_heading, parts.join("\n\n")));
    }
    sections.extend(blocks.iter().filter(|b| b.sop.is_none() && b.bottom).map(|b| b.text.clone()));
    sections.retain(|s| !s.is_empty());
    let values = config.variables.merged(&agent.variables);
    let mut tool_payload = Map::new();
    for s in sops.iter().filter(|s| s.delivery != "prompt") {
        let mut payload = sop_payload(s);
        fill_json(&mut payload, &values);
        tool_payload.insert(s.id.clone(), payload);
    }
    let tools: BTreeSet<&str> = sops.iter().flat_map(|s| sop_tools(s)).collect();
    Expected {
        prompt: fill_variables(&sections.join("\n\n"), &values) + "\n",
        tool_payload,
        tools: tools.into_iter().map(String::from).collect(),
    }
}

// --- editing YAML as text ----------------------------------------------------------------------

/// A top-level key in YAML text: the comment lines right above it (no blank line between), its
/// line, and the lines that continue its value. Line ranges are [start, end).
struct Span {
    key: String,
    start: usize,
    end: usize,
}

fn spans(lines: &[&str]) -> Vec<Span> {
    let mut out: Vec<Span> = vec![];
    let mut i = 0;
    while i < lines.len() {
        let Some(c) = KEY.captures(lines[i]) else {
            i += 1;
            continue;
        };
        let key = c[1].trim_matches(['"', '\'']).to_string();
        let mut start = i;
        let floor = out.last().map_or(0, |s| s.end);
        while start > floor && lines[start - 1].starts_with('#') {
            start -= 1;
        }
        let (mut end, mut j) = (i + 1, i + 1);
        while j < lines.len() {
            let l = lines[j];
            if l.trim().is_empty() {
                j += 1;
            } else if l.starts_with([' ', '\t']) || l.starts_with('-') {
                j += 1;
                end = j;
            } else {
                break;
            }
        }
        out.push(Span { key, start, end });
        i = end;
    }
    out
}

/// YAML text without the given top-level keys (and the comments right above them). A blank
/// line left on both sides of a removed key becomes one.
fn remove_keys(text: &str, keys: &[&str]) -> String {
    let mut lines: Vec<&str> = text.lines().collect();
    for s in spans(&lines).into_iter().rev().filter(|s| keys.contains(&s.key.as_str())) {
        let blank = |i: usize| lines.get(i).map_or(true, |l| l.trim().is_empty());
        let mut start = s.start;
        if (start == 0 || blank(start - 1)) && blank(s.end) && s.end < lines.len() {
            if start > 0 {
                start -= 1;
            } else {
                lines.remove(s.end);
            }
        }
        lines.drain(start..s.end);
    }
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

/// Markdown with the given keys removed from its front matter; the front matter goes when
/// nothing is left in it.
fn remove_front_matter_keys(text: &str, keys: &[&str]) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.first() != Some(&"---") {
        return text.to_string();
    }
    let Some(close) = lines.iter().skip(1).position(|l| l.starts_with("---")).map(|i| i + 1) else {
        return text.to_string();
    };
    let meta = remove_keys(&lines[1..close].join("\n"), keys);
    let body = lines[close + 1..].join("\n");
    let end = if text.ends_with('\n') { "\n" } else { "" };
    if meta.trim().is_empty() {
        return format!("{}{end}", body.trim_start_matches('\n'));
    }
    format!("---\n{}\n{}\n{body}{end}", meta.trim_start_matches('\n'), lines[close])
}

/// Whether two mappings hold the same keys and values, in any order.
fn same(a: &Mapping, b: &Mapping) -> bool {
    a.len() == b.len() && a.iter().all(|(k, v)| b.get(k) == Some(v))
}

fn without(map: &Mapping, keys: &[&str]) -> Mapping {
    map.iter()
        .filter(|(k, _)| !k.as_str().is_some_and(|k| keys.contains(&k)))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// Checks an edited file: its mapping must be the old one minus `removed`, plus `added`.
fn check_edit(path: &str, old: &Mapping, new_meta: &str, removed: &[&str], added: &Mapping, issues: &mut Vec<Issue>) {
    let mut want = without(old, removed);
    want.extend(added.iter().map(|(k, v)| (k.clone(), v.clone())));
    let mut scratch = vec![];
    match load_yaml(new_meta, path, &mut scratch) {
        Some(got) if same(&got, &want) => {}
        _ => issues.push(failed(path, "can't edit this file automatically; migrate it by hand (see FORMAT.md)")),
    }
}

fn front_matter(text: &str) -> &str {
    split_front_matter(text).0
}

fn yaml_list(key: &str, items: &[String]) -> String {
    let mut out = format!("{key}:");
    for item in items {
        out.push_str(&format!("\n  - {}", scalar(item, false)));
    }
    out
}

// --- migrating -----------------------------------------------------------------------------------

/// Converts an old-format folder (relative path → text), or removes fields the format no longer
/// has from a current one. Fails without changing anything when a file can't be read or edited
/// exactly.
pub fn migrate(files: &BTreeMap<String, String>) -> Result<Migration, Issues> {
    if is_old(files) {
        migrate_old(files)
    } else {
        remove_fields(files)
    }
}

fn migrate_old(files: &BTreeMap<String, String>) -> Result<Migration, Issues> {
    let mut issues = vec![];
    let in_folder = |folder: &'static str| {
        files.iter().filter(move |(p, _)| p.starts_with(folder) && p[folder.len()..].starts_with('/'))
    };
    if in_folder("instructions").next().is_some() {
        let msg = "instructions/ already exists: this folder is partly migrated; finish it by hand (see FORMAT.md)";
        return Err(Issues(vec![failed("instructions", msg)]));
    }
    let Some(config_text) = files.get(CONFIG) else {
        return Err(crate::workspace::missing_config(files));
    };
    let mut m = Migration::default();
    let mut new_files: BTreeMap<String, String> = files.clone();
    let mut edit = |m: &mut Migration, path: &str, new_path: &str, text: String, what: String| {
        let old = &files[path];
        let lost = lost_comments(path, old, &text);
        if !lost.is_empty() {
            m.lost_comments.push((path.to_string(), lost.iter().map(|(n, c)| format!("line {n}: # {c}")).collect()));
        }
        if new_path != path {
            new_files.remove(path);
            m.removes.push(path.to_string());
        } else if text == *old {
            return;
        }
        m.changes.push((path.to_string(), new_path.to_string(), what));
        new_files.insert(new_path.to_string(), text.clone());
        m.writes.push((new_path.to_string(), text));
    };
    fn removed_note(map: &Mapping, keys: &[&str]) -> String {
        let present: Vec<&str> = keys.iter().copied().filter(|k| map.contains_key(*k)).collect();
        if present.is_empty() {
            String::new()
        } else {
            format!(": removed {}", present.join(", "))
        }
    }

    // sop.yaml
    let Some(config_map) = load_yaml(config_text, CONFIG, &mut issues) else { return Err(Issues(issues)) };
    let order = texts(&config_map, "sop_order");
    let config_new = remove_keys(config_text, model::OLD_CONFIG_FIELDS) + "\n";
    check_edit(CONFIG, &config_map, &config_new, model::OLD_CONFIG_FIELDS, &Mapping::new(), &mut issues);
    let config = match model::parse_config(&without(&config_map, model::OLD_CONFIG_FIELDS)) {
        Ok(c) => c,
        Err(errs) => {
            issues.extend(errs.into_iter().map(|e| failed(CONFIG, e)));
            model::Config::default()
        }
    };

    // bases and SOPs
    let mut bases = vec![];
    let mut base_meta = BTreeMap::new();
    for (path, text) in in_folder("bases") {
        let Some(map) = load_yaml(front_matter(text), path, &mut issues) else { continue };
        bases.push(old_block(stem(path), &map, None, split_front_matter(text).1.trim().to_string()));
        base_meta.insert(path.clone(), map);
    }
    let mut sops = vec![];
    let mut sop_meta = BTreeMap::new();
    for (path, text) in in_folder("procedures") {
        let markdown = path.ends_with(".md");
        let meta = if markdown { front_matter(text) } else { text.as_str() };
        let Some(map) = load_yaml(meta, path, &mut issues) else { continue };
        let stripped = if markdown {
            remove_front_matter_keys(text, &old_sop_keys())
        } else {
            remove_keys(text, &old_sop_keys()) + "\n"
        };
        match parse_sop_file(path, &stripped) {
            Ok((sop, _)) => sops.push(old_block(stem(path), &map, Some(sop), String::new())),
            Err(errs) => issues.extend(errs),
        }
        sop_meta.insert(path.clone(), (map, stripped));
    }

    // agents
    let mut agents = vec![];
    let mut agent_maps = BTreeMap::new();
    for (path, text) in in_folder("agents") {
        let Some(map) = load_yaml(text, path, &mut issues) else { continue };
        if map.contains_key("blocks") || map.contains_key("context") {
            issues.push(failed(
                path,
                "already has `blocks` or `context`: this folder is partly migrated; finish it by hand",
            ));
            continue;
        }
        let mut new_map = without(&map, model::OLD_AGENT_FIELDS);
        new_map.insert("id".into(), stem(path).into());
        if let Some(v) = map.get("instructions") {
            new_map.insert("context".into(), v.clone());
        }
        match model::parse_agent(&new_map) {
            Ok(agent) => {
                agents.push(OldAgent { agent, inherits: texts(&map, "inherits"), exclude: texts(&map, "exclude") })
            }
            Err(errs) => issues.extend(errs.into_iter().map(|e| failed(path, e))),
        }
        agent_maps.insert(path.clone(), map);
    }
    for b in &bases {
        for parent in b.inherits.iter().filter(|p| !bases.iter().any(|x| &x.id == *p)) {
            issues.push(failed(&format!("bases/{}.md", b.id), format!("inherits '{parent}', which is not a base")));
        }
    }
    for a in &agents {
        for id in a.inherits.iter().filter(|p| !bases.iter().any(|x| &x.id == *p)) {
            issues
                .push(failed(&format!("agents/{}.yaml", a.agent.id), format!("inherits '{id}', which is not a base")));
        }
    }
    if !issues.is_empty() {
        return Err(Issues(issues));
    }

    // Each agent's blocks under the old rules.
    let mut compositions: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for a in &agents {
        let blocks = old_composition(&bases, &sops, &order, a);
        m.expected.insert(a.agent.id.clone(), expected(&blocks, &a.agent, &config));
        compositions.insert(a.agent.id.clone(), blocks.iter().map(|b| b.id.clone()).collect());
    }

    // The edits.
    edit(&mut m, CONFIG, CONFIG, config_new, removed_note(&config_map, model::OLD_CONFIG_FIELDS));
    for (path, map) in &base_meta {
        let id = stem(path);
        let keys = [model::OLD_INSTRUCTION_FIELDS, model::REMOVED_FIELDS].concat();
        let text = remove_front_matter_keys(&files[path], &keys);
        check_edit(path, map, front_matter(&text), &keys, &Mapping::new(), &mut issues);
        edit(&mut m, path, &format!("instructions/{id}.md"), text, removed_note(map, &keys));
    }
    for (path, (map, text)) in &sop_meta {
        let keys = old_sop_keys();
        let meta = if path.ends_with(".md") { front_matter(text) } else { text.as_str() };
        check_edit(path, map, meta, &keys, &Mapping::new(), &mut issues);
        edit(&mut m, path, path, text.clone(), removed_note(map, &keys));
    }
    for (path, map) in &agent_maps {
        let blocks = &compositions[stem(path)];
        let context = map.get("instructions").and_then(Value::as_str).unwrap_or_default();
        let text = agent_text(&files[path], context, blocks);
        let mut added = Mapping::new();
        if let Some(v) = map.get("instructions") {
            added.insert("context".into(), v.clone());
        }
        if !blocks.is_empty() {
            added.insert("blocks".into(), Value::Sequence(blocks.iter().map(|b| Value::from(b.as_str())).collect()));
        }
        check_edit(path, map, &text, model::OLD_AGENT_FIELDS, &added, &mut issues);
        let mut notes = vec![];
        if map.contains_key("instructions") {
            notes.push("instructions → context".to_string());
        }
        let gone: Vec<&str> = ["inherits", "exclude"].into_iter().filter(|k| map.contains_key(*k)).collect();
        if !gone.is_empty() {
            notes.push(format!("removed {}", gone.join(", ")));
        }
        notes.push(format!("blocks: {}", if blocks.is_empty() { "none".into() } else { blocks.join(", ") }));
        edit(&mut m, path, path, text, format!(": {}", notes.join("; ")));
    }
    if !issues.is_empty() {
        return Err(Issues(issues));
    }
    m.changes.sort();
    m.files = new_files;
    Ok(m)
}

/// The fields an old SOP file loses.
fn old_sop_keys() -> Vec<&'static str> {
    [model::OLD_SOP_FIELDS, model::REMOVED_FIELDS].concat()
}

/// Removes fields the format no longer has (`locked`) from a folder that is otherwise current.
/// They never reached a prompt, so every prompt stays the same; that is checked by building it.
fn remove_fields(files: &BTreeMap<String, String>) -> Result<Migration, Issues> {
    let mut issues = vec![];
    let mut m = Migration { removed_fields_only: true, ..Migration::default() };
    let mut new_files = files.clone();
    let keys = model::REMOVED_FIELDS;
    let blocks = files
        .iter()
        .filter(|(p, _)| (p.starts_with("instructions/") && p.ends_with(".md")) || p.starts_with("procedures/"));
    for (path, text) in blocks {
        let markdown = path.ends_with(".md");
        let meta = if markdown { front_matter(text) } else { text.as_str() };
        let Some(map) = load_yaml(meta, path, &mut issues) else { continue };
        if !keys.iter().any(|k| map.contains_key(*k)) {
            continue;
        }
        let new = if markdown { remove_front_matter_keys(text, keys) } else { remove_keys(text, keys) + "\n" };
        let new_meta = if markdown { front_matter(&new) } else { new.as_str() };
        check_edit(path, &map, new_meta, keys, &Mapping::new(), &mut issues);
        let lost = lost_comments(path, text, &new);
        if !lost.is_empty() {
            m.lost_comments.push((path.clone(), lost.iter().map(|(n, c)| format!("line {n}: # {c}")).collect()));
        }
        m.changes.push((path.clone(), path.clone(), format!(": removed {}", keys.join(", "))));
        m.writes.push((path.clone(), new.clone()));
        new_files.insert(path.clone(), new);
    }
    if !issues.is_empty() {
        return Err(Issues(issues));
    }
    let build = render_workspace(&load_files(&new_files)?)?;
    for (id, r) in build.agents {
        let e = Expected { prompt: r.prompt, tool_payload: r.tool_payload, tools: r.tools };
        m.expected.insert(id, e);
    }
    m.files = new_files;
    Ok(m)
}

/// An agent file with `instructions`, `inherits` and `exclude` removed, and `context` and
/// `blocks` right after the platform id.
fn agent_text(text: &str, context: &str, blocks: &[String]) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let platform = spans(&lines).into_iter().find(|s| model::PLATFORMS.contains(&s.key.as_str()));
    let Some(platform) = platform else { return text.to_string() };
    let key_line = (platform.start..platform.end).find(|i| KEY.is_match(lines[*i])).unwrap_or(platform.start);
    let before = remove_keys(&lines[..key_line].join("\n"), model::OLD_AGENT_FIELDS);
    let after = remove_keys(&lines[platform.end..].join("\n"), model::OLD_AGENT_FIELDS);
    let mut out = vec![];
    if !before.trim().is_empty() {
        out.push(before.trim_end().to_string());
        if lines[..key_line].last().is_some_and(|l| l.trim().is_empty()) {
            out.push(String::new());
        }
    }
    out.push(lines[key_line..platform.end].join("\n"));
    if !context.is_empty() {
        out.push(kv("context", context, 0).trim_end_matches('\n').to_string());
    }
    if !blocks.is_empty() {
        out.push(yaml_list("blocks", blocks));
    }
    let after = after.trim_start_matches('\n');
    if !after.trim().is_empty() {
        out.extend([String::new(), after.to_string()]);
    }
    out.join("\n") + "\n"
}

/// Builds the migrated files and checks every agent builds what it should: its old prompt with
/// its own text at the top, the same get_sop payloads and the same tools. Returns the number of
/// agents checked.
pub fn verify(files: &BTreeMap<String, String>, expected: &BTreeMap<String, Expected>) -> Result<usize, Issues> {
    let build = render_workspace(&load_files(files)?)?;
    let mut issues = vec![];
    let ids: BTreeSet<&String> = build.agents.keys().chain(expected.keys()).collect();
    for id in ids {
        let path = format!("agents/{id}.yaml");
        let (Some(r), Some(want)) = (build.agents.get(id), expected.get(id)) else {
            issues.push(failed(&path, "the agent is missing before or after migrating"));
            continue;
        };
        let got = Expected { prompt: r.prompt.clone(), tool_payload: r.tool_payload.clone(), tools: r.tools.clone() };
        if got.prompt != want.prompt {
            let diff = crate::plan::labelled_diff(&want.prompt, &got.prompt, "expected", "migrated");
            issues.push(failed(&path, format!("the migrated prompt differs from the old one:\n{diff}")));
        } else if got != *want {
            issues.push(failed(&path, "the migrated get_sop payloads or tools differ from the old ones"));
        }
    }
    if issues.is_empty() {
        Ok(expected.len())
    } else {
        Err(Issues(issues))
    }
}

/// Migrates a file map in memory if it needs it, for building a git ref from before the change.
pub fn convert(files: BTreeMap<String, String>) -> Result<BTreeMap<String, String>, Issues> {
    if !needs_migrating(&files) {
        return Ok(files);
    }
    Ok(migrate(&files)?.files)
}

/// The plan `sop migrate` prints; `show` turns a path in the folder into one to print.
pub fn plan_text(m: &Migration, root: &str, show: &dyn Fn(&str) -> String) -> String {
    let n = m.changes.len();
    let mut out = vec![if m.removed_fields_only {
        format!("Removing fields the format no longer has from {root} ({n} file(s) change):")
    } else {
        format!("Migrating {root} to the current format ({n} file(s) change):")
    }];
    for (path, new_path, what) in &m.changes {
        let moved = if new_path == path { String::new() } else { format!(" → {}", show(new_path)) };
        out.push(format!("  {}{moved}{what}", show(path)));
    }
    if !m.lost_comments.is_empty() {
        out.push("Comments that go with removed fields:".into());
        for (path, lines) in &m.lost_comments {
            out.push(format!("  {}", show(path)));
            out.extend(lines.iter().map(|l| format!("    {l}")));
        }
    }
    out.push(if m.removed_fields_only {
        format!("Checked: all {} agent prompt(s) stay the same.", m.expected.len())
    } else {
        format!(
            "Checked: all {} agent prompt(s) stay the same, except each agent's own text (now `context`) moves to the top.",
            m.expected.len()
        )
    });
    out.join("\n") + "\n"
}
