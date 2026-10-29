//! SOP files in Markdown (procedures/<id>.md), and the canonical text of both SOP formats that
//! `sopc fmt` and `sopc convert` write.
//!
//! A Markdown SOP is optional front matter with settings only, one `# name` heading, the
//! `**Goal:**` and `**When:**` fields, guidance paragraphs, and the `## Steps`, `## Never` and
//! `## Warning signs` sections as lists. It is read into the same mapping a YAML SOP file holds.

use crate::model::{Sop, Step};
use crate::render::sop_payload;
use crate::workspace::{load_yaml, parse_sop_file, split_front_matter, stem, Issue};
use regex::Regex;
use serde_yaml_ng::{Mapping, Value};
use std::sync::LazyLock;

/// Front matter keys a Markdown SOP may set; everything else is written in the body.
pub const SETTINGS: &[&str] = &["id", "delivery"];
/// (heading, field), in the standard order.
const SECTIONS: [(&str, &str); 3] =
    [("Steps", "procedureSteps"), ("Never", "forbiddenActions"), ("Warning signs", "warningSigns")];
/// (label, field).
const FIELDS: [(&str, &str); 2] = [("Goal", "description"), ("When", "scope")];

static H1: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^#\s+(.*?)\s*$").unwrap());
static H2: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^##(?:\s+(.*?))?\s*$").unwrap());
static DEEPER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^#{3,}(?:\s|$)").unwrap());
static LABEL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\*\*([^*]+?)(:\*\*|\*\*:)\s*(.*)$").unwrap());
static ITEM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:-|\d+\.)(?:\s+(.*))?$").unwrap());
static ANNOTATION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*`([^`]*)`\s*$").unwrap());
/// An `OR` option line under a step: `OR <text>`, indented or not.
static OR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^OR\s+(.+)$").unwrap());
static TOOL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^tool:\s*(\S+)$").unwrap());

fn section_list() -> &'static str {
    "## Steps, ## Never and ## Warning signs"
}

/// Collects one file's problems, with line numbers.
struct Report<'a> {
    path: &'a str,
    issues: &'a mut Vec<Issue>,
    warnings: &'a mut Vec<Issue>,
}

impl Report<'_> {
    fn error(&mut self, line: usize, code: &'static str, msg: impl std::fmt::Display) {
        self.issues.push(Issue::error(code, self.path, format!("line {line}: {msg}")));
    }
    fn warning(&mut self, line: usize, code: &'static str, msg: impl std::fmt::Display) {
        self.warnings.push(Issue::warning(code, self.path, format!("line {line}: {msg}")));
    }
}

#[derive(Clone, Copy, PartialEq)]
enum At {
    Head,
    Section(usize),
    Unknown,
}

/// Reads procedures/<id>.md into the mapping a YAML SOP file would hold.
pub fn parse_markdown(text: &str, path: &str, issues: &mut Vec<Issue>, warnings: &mut Vec<Issue>) -> Option<Mapping> {
    let before = issues.len();
    let (meta, body) = split_front_matter(text);
    let first = text[..text.len() - body.len()].matches('\n').count() + 1; // line number of the body
    let mut r = Report { path, issues, warnings };
    let mut map = load_yaml(meta, path, r.issues)?;
    crate::workspace::removed_fields(&mut map, path, r.issues);
    let fields: Vec<String> = map.keys().filter_map(|k| k.as_str()).map(String::from).collect();
    for key in fields.iter().filter(|k| !SETTINGS.contains(&k.as_str())) {
        if crate::model::OLD_SOP_FIELDS.contains(&key.as_str()) {
            r.issues.push(crate::workspace::old_format(path, &format!("`{key}`")));
            map.remove(key.as_str());
            continue;
        }
        let line = meta.lines().position(|l| l.trim_start_matches(['"', '\'']).starts_with(key.as_str()));
        let hint = match key.as_str() {
            "name" => "write the name as the `# <name>` heading".to_string(),
            "description" => "write it as `**Goal:**` under the name".into(),
            "scope" => "write it as `**When:**` under the name".into(),
            "guidance" => "write it as paragraphs under the name".into(),
            k => match SECTIONS.iter().find(|(_, f)| *f == k) {
                Some((h, _)) => format!("write it as a `## {h}` list"),
                None => format!("front matter holds only {}", SETTINGS.join(", ")),
            },
        };
        r.error(line.map_or(1, |n| n + 2), "md_settings_field", format!("`{key}` belongs in the body: {hint}"));
        map.remove(key.as_str());
    }

    let mut name: Option<(usize, String)> = None;
    let mut values: [Option<String>; 2] = [None, None]; // Goal, When
    let (mut field, mut paragraphs, mut para) = (None::<usize>, vec![], Vec::<&str>::new());
    let mut sections: [Option<(usize, Vec<Value>)>; 3] = [None, None, None];
    // The open list item: its line and text, then each `OR` option's line and text.
    let (mut at, mut title, mut item) = (At::Head, String::new(), None::<Vec<(usize, String)>>);
    let mut reported_before = false;

    let close_item = |item: &mut Option<Vec<(usize, String)>>,
                      at: At,
                      r: &mut Report,
                      sections: &mut [Option<(usize, Vec<Value>)>; 3]| {
        if let (Some(parts), At::Section(k)) = (item.take(), at) {
            let n = parts[0].0;
            if let Some(v) = parse_step(&parts, k, r) {
                sections[k].get_or_insert((n, vec![])).1.push(v);
            }
        }
    };
    for (i, raw) in body.lines().enumerate() {
        let n = first + i;
        let line = raw.trim_end();
        let t = line.trim_start();
        if t.is_empty() {
            if at == At::Head {
                paragraphs.extend((!para.is_empty()).then(|| para.join("\n")));
                (para, field) = (vec![], None);
            }
            continue;
        }
        if name.is_none() && !H1.is_match(t) && !reported_before {
            reported_before = true;
            r.error(n, "md_text_before_name", "text before the `# <name>` heading; the name must come first");
        }
        if DEEPER.is_match(t) {
            let msg = format!(
                "`{}` heading; only `# <name>` and {} are allowed",
                t.split(' ').next().unwrap_or(t),
                section_list()
            );
            r.error(n, "md_heading_level", msg);
        } else if let Some(c) = H2.captures(t) {
            close_item(&mut item, at, &mut r, &mut sections);
            paragraphs.extend((!para.is_empty()).then(|| para.join("\n")));
            (para, field) = (vec![], None);
            title = c.get(1).map_or("", |m| m.as_str()).to_string();
            at = match SECTIONS.iter().position(|(h, _)| *h == title) {
                None => {
                    r.error(
                        n,
                        "md_unknown_section",
                        format!("unknown section `## {title}`; the sections are {}", section_list()),
                    );
                    At::Unknown
                }
                Some(k) => {
                    if sections[k].is_some() {
                        r.error(n, "md_duplicate_section", format!("`## {title}` appears twice; merge them into one"));
                    }
                    sections[k].get_or_insert((n, vec![]));
                    At::Section(k)
                }
            };
        } else if let Some(c) = H1.captures(t) {
            match &name {
                Some((first, _)) => r.error(
                    n,
                    "md_extra_name",
                    format!("a second `# ` heading (the name is on line {first}); sections are {}", section_list()),
                ),
                None => name = Some((n, c[1].to_string())),
            }
        } else if at != At::Head {
            let indented = raw.starts_with([' ', '\t']);
            let new_item = if indented { None } else { ITEM.captures(line) };
            let option = OR.captures(t).filter(|_| item.is_some());
            if let Some(c) = new_item {
                close_item(&mut item, at, &mut r, &mut sections);
                item = Some(vec![(n, c.get(1).map_or("", |m| m.as_str()).to_string())]);
            } else if let (Some(c), Some(parts)) = (option, item.as_mut()) {
                parts.push((n, c[1].to_string()));
            } else if let (true, Some(parts)) = (indented, item.as_mut()) {
                let text = &mut parts.last_mut().unwrap().1;
                text.push(' ');
                text.push_str(t);
            } else {
                r.error(
                    n,
                    "md_text_in_section",
                    format!("only list items (`- ` or `1. `) can go under `## {title}`; put other text above the first `##` section"),
                );
            }
        } else if let Some(c) = LABEL.captures(t) {
            paragraphs.extend((!para.is_empty()).then(|| para.join("\n")));
            (para, field) = (vec![], None);
            match FIELDS.iter().position(|(l, _)| *l == &c[1] && &c[2] == ":**") {
                None => r.error(
                    n,
                    "md_unknown_field",
                    format!("unknown field `**{}:**`; the fields are `**Goal:**` and `**When:**`", &c[1]),
                ),
                Some(k) if values[k].is_some() => {
                    r.error(n, "md_duplicate_field", format!("`**{}:**` is given twice", FIELDS[k].0))
                }
                Some(k) => {
                    values[k] = Some(c[3].to_string());
                    field = Some(k);
                }
            }
        } else if let Some(k) = field {
            let v = values[k].get_or_insert_with(String::new);
            v.push(' ');
            v.push_str(t);
        } else {
            para.push(line);
        }
    }
    close_item(&mut item, at, &mut r, &mut sections);
    paragraphs.extend((!para.is_empty()).then(|| para.join("\n")));

    let Some((name_line, name)) = name else {
        r.error(first, "md_missing_name", "no `# <name>` heading; start the SOP with one");
        return None;
    };
    match &sections[0] {
        None => r.error(name_line, "missing_steps", "no `## Steps` section; every SOP needs at least one step"),
        Some((n, items)) if items.is_empty() => {
            r.error(*n, "missing_steps", "`## Steps` has no items; every SOP needs at least one step")
        }
        _ => {}
    }
    for (k, (h, _)) in SECTIONS.iter().enumerate().skip(1) {
        if let Some((n, items)) = &sections[k] {
            if items.is_empty() {
                r.warning(*n, "md_empty_section", format!("`## {h}` has no items; add some or remove the heading"));
            }
        }
    }
    if r.issues.len() > before {
        return None;
    }
    map.insert("name".into(), name.into());
    for ((_, key), value) in FIELDS.iter().zip(values) {
        map.extend(value.map(|v| (Value::from(*key), Value::from(v.trim()))));
    }
    if !paragraphs.is_empty() {
        map.insert("guidance".into(), paragraphs.join("\n\n").into());
    }
    for ((_, key), section) in SECTIONS.iter().zip(sections) {
        map.extend(section.map(|(_, items)| (Value::from(*key), Value::Sequence(items))));
    }
    Some(map)
}

/// A list item with its `OR` options: a string, or {text, tool, required, or}.
fn parse_step(parts: &[(usize, String)], section: usize, r: &mut Report) -> Option<Value> {
    let (n, text) = &parts[0];
    let first = parse_item(text, *n, section, r);
    if parts.len() == 1 {
        return first;
    }
    if section != 0 {
        let msg = format!("`OR` options only apply under ## Steps, not ## {}", SECTIONS[section].0);
        r.error(parts[1].0, "md_or_outside_steps", msg);
        return None;
    }
    let options: Vec<Value> = parts[1..].iter().filter_map(|(n, t)| parse_item(t, *n, section, r)).collect();
    let mut m = match first? {
        Value::Mapping(m) => m,
        v => Mapping::from_iter([(Value::from("text"), v)]),
    };
    m.insert("or".into(), Value::Sequence(options));
    Some(Value::Mapping(m))
}

/// A list item's text with its trailing `tool: name` and `required` markers.
fn parse_item(text: &str, n: usize, section: usize, r: &mut Report) -> Option<Value> {
    let mut text = text.trim_end().to_string();
    let (mut tool, mut required) = (None::<String>, false);
    while let Some(c) = ANNOTATION.captures(&text) {
        let inner = c[1].trim().to_string();
        let start = c.get(0).unwrap().start();
        if inner == "required" {
            required = true;
        } else if let (Some(m), None) = (TOOL.captures(&inner), &tool) {
            tool = Some(m[1].to_string());
        } else if inner.starts_with("tool") || inner.starts_with("Tool") || TOOL.is_match(&inner) {
            let msg = format!("`{inner}` isn't a tool marker; write one `tool: <name>` with the tool's exact name");
            r.error(n, "md_bad_annotation", msg);
        } else {
            break;
        }
        text.truncate(start);
    }
    if required && section != 0 {
        r.error(
            n,
            "md_required_outside_steps",
            format!("`required` only applies under ## Steps, not ## {}", SECTIONS[section].0),
        );
    }
    let text = text.trim();
    if text.is_empty() {
        r.error(n, "empty_step", "a list item with no text");
        return None;
    }
    if tool.is_none() && !required {
        return Some(text.into());
    }
    let mut m = Mapping::new();
    m.insert("text".into(), text.into());
    m.extend(tool.map(|t| (Value::from("tool"), Value::from(t))));
    if required {
        m.insert("required".into(), true.into());
    }
    Some(Value::Mapping(m))
}

// --- writing --------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Yaml,
    Markdown,
}

impl Kind {
    pub fn of(path: &str) -> Kind {
        if path.ends_with(".md") {
            Kind::Markdown
        } else {
            Kind::Yaml
        }
    }
    fn ext(self) -> &'static str {
        match self {
            Kind::Yaml => "yaml",
            Kind::Markdown => "md",
        }
    }
    pub fn path(self, id: &str) -> String {
        format!("procedures/{id}.{}", self.ext())
    }
}

/// A YAML scalar on one line: plain when YAML reads it back unchanged, else double-quoted.
pub fn scalar(s: &str, flow: bool) -> String {
    let plain = !s.is_empty()
        && s.trim() == s
        && !(flow && s.contains([',', '[', ']', '{', '}']))
        && serde_yaml_ng::from_str::<Value>(s).ok() == Some(Value::String(s.to_string()));
    if plain {
        s.to_string()
    } else {
        serde_json::to_string(s).unwrap()
    }
}

/// `key: value` at an indent; text with line breaks as a `|` block.
pub fn kv(key: &str, value: &str, indent: usize) -> String {
    let body = value.trim_end_matches('\n');
    let printable = !value.chars().any(|c| c.is_control() && c != '\n');
    if !value.contains('\n') || body.is_empty() || body.starts_with(' ') || !printable {
        return format!("{key}: {}", scalar(value, false));
    }
    let chomp = match value.len() - body.len() {
        0 => "-",
        1 => "",
        _ => "+",
    };
    let pad = " ".repeat(indent + 2);
    let mut lines: Vec<String> =
        body.split('\n').map(|l| if l.is_empty() { String::new() } else { format!("{pad}{l}") }).collect();
    lines.extend(std::iter::repeat(String::new()).take((value.len() - body.len()).saturating_sub(1)));
    format!("{key}: |{chomp}\n{}", lines.join("\n"))
}

/// The settings that differ from their defaults, as YAML lines.
fn settings(sop: &Sop) -> Vec<String> {
    let mut out = vec![];
    if sop.delivery != "prompt" {
        out.push(format!("delivery: {}", sop.delivery));
    }
    out
}

fn step_md(s: &Step) -> String {
    let mut out = s.text.trim().to_string();
    if let Some(tool) = &s.tool {
        out += &format!(" `tool: {tool}`");
    }
    if s.required {
        out += " `required`";
    }
    for o in &s.or {
        out += &format!("\n   OR {}", step_md(o));
    }
    out
}

/// An SOP as Markdown; `front` is the front matter between the `---` lines.
pub fn to_markdown(sop: &Sop, front: &str) -> String {
    let mut blocks = vec![];
    if !front.trim().is_empty() {
        blocks.push(format!("---\n{}\n---\n# {}", front.trim_matches('\n'), sop.name.trim()));
    } else {
        blocks.push(format!("# {}", sop.name.trim()));
    }
    let fields: Vec<String> = [("Goal", &sop.description), ("When", &sop.scope)]
        .iter()
        .filter(|(_, v)| !v.trim().is_empty())
        .map(|(l, v)| format!("**{l}:** {}", v.trim()))
        .collect();
    blocks.extend((!fields.is_empty()).then(|| fields.join("\n")));
    blocks.extend((!sop.guidance.trim().is_empty()).then(|| sop.guidance.trim().to_string()));
    for (k, (heading, list)) in
        [("Steps", &sop.procedure_steps), ("Never", &sop.forbidden_actions), ("Warning signs", &sop.warning_signs)]
            .into_iter()
            .enumerate()
    {
        if list.is_empty() {
            continue;
        }
        let items = list.iter().enumerate().map(|(i, s)| {
            let marker = if k == 0 { format!("{}.", i + 1) } else { "-".into() };
            format!("{marker} {}", step_md(s))
        });
        blocks.push(format!("## {heading}\n{}", items.collect::<Vec<_>>().join("\n")));
    }
    blocks.join("\n\n") + "\n"
}

/// An SOP as YAML; `header` is comment lines kept at the top.
pub fn to_yaml(sop: &Sop, header: &str) -> String {
    let mut lines = vec![kv("name", &sop.name, 0)];
    lines.extend(settings(sop));
    for (key, value) in [("description", &sop.description), ("scope", &sop.scope), ("guidance", &sop.guidance)] {
        if !value.is_empty() {
            lines.push(kv(key, value, 0));
        }
    }
    for (key, list) in [
        ("procedureSteps", &sop.procedure_steps),
        ("forbiddenActions", &sop.forbidden_actions),
        ("warningSigns", &sop.warning_signs),
    ] {
        if list.is_empty() {
            continue;
        }
        lines.push(format!("{key}:"));
        for s in list {
            if s.plain {
                lines.push(kv("  -", &s.text, 2).replacen("  -: ", "  - ", 1));
                continue;
            }
            lines.push(kv("  - text", &s.text, 4));
            if let Some(tool) = &s.tool {
                lines.push(format!("    tool: {}", scalar(tool, false)));
            }
            if s.required {
                lines.push("    required: true".into());
            }
            if !s.or.is_empty() {
                lines.push("    or:".into());
            }
            for o in &s.or {
                if o.plain {
                    lines.push(kv("      -", &o.text, 6).replacen("      -: ", "      - ", 1));
                    continue;
                }
                lines.push(kv("      - text", &o.text, 8));
                if let Some(tool) = &o.tool {
                    lines.push(format!("        tool: {}", scalar(tool, false)));
                }
                if o.required {
                    lines.push("        required: true".into());
                }
            }
        }
    }
    let header = header.trim_matches('\n');
    let sep = if header.is_empty() { "" } else { "\n\n" };
    format!("{header}{sep}{}\n", lines.join("\n"))
}

/// Comment lines at the top of a YAML file, up to the first other line.
fn yaml_header(text: &str) -> String {
    let lines = text.lines().take_while(|l| l.trim().is_empty() || l.trim_start().starts_with('#'));
    lines.collect::<Vec<_>>().join("\n").trim_matches('\n').to_string()
}

/// Comment lines carried over from one format to the other (not editor schema hints).
fn comments(text: &str) -> Vec<&str> {
    let lines: Vec<&str> =
        text.lines().filter(|l| l.trim_start().starts_with('#') && !l.contains("yaml-language-server")).collect();
    let start = lines.iter().position(|l| l.trim() != "#").unwrap_or(lines.len());
    let end = lines.iter().rposition(|l| l.trim() != "#").map_or(start, |i| i + 1);
    lines[start..end.max(start)].to_vec()
}

/// Comments in `text` (whole-line or trailing) that `new_text` no longer contains, as
/// (line number, comment). Editor schema hints are ignored: they belong to one format.
pub fn lost_comments(path: &str, text: &str, new_text: &str) -> Vec<(usize, String)> {
    let scope = match Kind::of(path) {
        Kind::Yaml => text,
        Kind::Markdown => split_front_matter(text).0,
    };
    let offset = if Kind::of(path) == Kind::Markdown && text.starts_with("---") { 1 } else { 0 };
    scope
        .lines()
        .enumerate()
        .filter_map(|(i, line)| comment_of(line).map(|c| (i + 1 + offset, c)))
        .filter(|(_, c)| !c.contains("yaml-language-server") && !new_text.contains(c.as_str()))
        .collect()
}

/// The comment on a YAML line (text after a `#` that starts the line or follows whitespace,
/// outside quotes), trimmed, if there is one.
fn comment_of(line: &str) -> Option<String> {
    let (mut quote, mut prev) = (None, ' ');
    for (i, ch) in line.char_indices() {
        match (quote, ch) {
            (None, '"' | '\'') => quote = Some(ch),
            (Some(q), c) if c == q => quote = None,
            (None, '#') if prev.is_whitespace() => {
                let c = line[i + 1..].trim();
                return (!c.is_empty()).then(|| c.to_string());
            }
            _ => {}
        }
        prev = ch;
    }
    None
}

/// What in two SOPs' rendered output differs, if anything.
fn output_difference(a: &Sop, b: &Sop) -> Option<String> {
    let (pa, pb) = (sop_payload(a), sop_payload(b));
    let keys = ["name", "description", "scope", "guidance", "procedureSteps", "forbiddenActions", "warningSigns"];
    if let Some(k) = keys.iter().find(|k| pa[**k] != pb[**k]) {
        return Some(k.to_string());
    }
    (a.delivery != b.delivery).then(|| "delivery".to_string())
}

/// The canonical text of an SOP file in a format: (new path, new text). Fails when the file
/// has errors, or when writing it that way would change what an agent is given.
pub fn rewrite(path: &str, text: &str, to: Kind) -> Result<(String, String), Vec<Issue>> {
    let (sop, _) = parse_sop_file(path, text)?;
    let from = Kind::of(path);
    let (front, header) = match from {
        Kind::Markdown => (split_front_matter(text).0.to_string(), comments(split_front_matter(text).0).join("\n")),
        Kind::Yaml => {
            let header = yaml_header(text);
            let mut front = comments(&header);
            let settings = settings(&sop);
            front.extend(settings.iter().map(String::as_str));
            (front.join("\n"), header.clone())
        }
    };
    let mut out_sop = sop.clone();
    let new_text = match to {
        Kind::Markdown => to_markdown(&sop, &front),
        Kind::Yaml => {
            if from == Kind::Markdown && !out_sop.guidance.is_empty() {
                out_sop.guidance.push('\n'); // a `|` block, as guidance is usually written
            }
            to_yaml(&out_sop, &header)
        }
    };
    let new_path = to.path(stem(path));
    let fail = |why: String| {
        let msg = format!("can't write it as {} without changing the built prompt ({why}); edit it by hand", to.ext());
        vec![Issue::error("convert_failed", path, msg)]
    };
    let (back, _) = parse_sop_file(&new_path, &new_text).map_err(|e| fail(e[0].message.clone()))?;
    let same = if from == to { back.canonical_json() == out_sop.canonical_json() } else { true };
    match output_difference(&sop, &back) {
        Some(field) => Err(fail(format!("{field} would change"))),
        None if !same => Err(fail("a value would change".into())),
        None => Ok((new_path, new_text)),
    }
}
