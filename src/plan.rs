//! Comparing builds: `plan` (which prompts change, and because of which blocks) and
//! `affected` (which agents to test).

use crate::render::Build;
use crate::text::ascii_json;
use crate::workspace::{read_text, Issue, Issues};
use serde_json::{json, Value as Json};
use similar::TextDiff;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// One agent's build, as plan sees it.
pub struct AgentSnapshot {
    pub prompt: String,
    pub platform_ref: String,
    pub blocks: BTreeMap<String, String>, // "kind:id" → hash
}

pub type Snapshot = BTreeMap<String, AgentSnapshot>;

pub fn snapshot(build: &Build) -> Snapshot {
    let snap = build.agents.iter().map(|(id, r)| {
        let blocks = r.block_hashes().into_iter().map(|(kind, id, hash)| (format!("{kind}:{id}"), hash)).collect();
        (id.clone(), AgentSnapshot { prompt: r.prompt.clone(), platform_ref: r.agent.platform_ref(), blocks })
    });
    snap.collect()
}

/// Reads a build/ folder; without lock.json it is empty.
pub fn read_snapshot(dir: &Path) -> anyhow::Result<Snapshot> {
    let Ok(text) = std::fs::read_to_string(dir.join("lock.json")) else { return Ok(Snapshot::new()) };
    let lock: Json = serde_json::from_str(&text)?;
    let mut snap = Snapshot::new();
    for (id, entry) in lock["agents"].as_object().into_iter().flatten() {
        let str_of = |v: &Json| v.as_str().unwrap_or_default().to_string();
        let blocks = entry["blocks"].as_array().into_iter().flatten();
        snap.insert(
            id.clone(),
            AgentSnapshot {
                prompt: read_text(&dir.join(format!("{id}.prompt.md")))?,
                platform_ref: str_of(&entry["platform_ref"]),
                blocks: blocks
                    .map(|b| (format!("{}:{}", str_of(&b["kind"]), str_of(&b["id"])), str_of(&b["hash"])))
                    .collect(),
            },
        );
    }
    Ok(snap)
}

/// An agent whose prompt differs.
pub struct AgentChange {
    pub agent: String,
    pub status: &'static str, // added | removed | changed
    pub platform_ref: String,
    pub blocks: BTreeMap<String, &'static str>, // "kind:id" → edited | added | removed
    pub diff: String,
}

pub struct Plan {
    pub changes: Vec<AgentChange>,
}

fn unified_diff(old: &str, new: &str, id: &str) -> String {
    labelled_diff(old, new, &format!("a/{id}.prompt.md"), &format!("b/{id}.prompt.md"))
}

/// A unified diff of two texts, with 3 lines of context and the given `---`/`+++` labels.
pub fn labelled_diff(old: &str, new: &str, old_label: &str, new_label: &str) -> String {
    let diff = TextDiff::from_lines(old, new);
    let mut udiff = diff.unified_diff();
    udiff.context_radius(3).missing_newline_hint(false).header(old_label, new_label);
    udiff.to_string()
}

pub fn make_plan(before: &Snapshot, after: &Snapshot) -> Plan {
    let ids: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    let mut changes = vec![];
    for id in ids {
        let (old, new) = (before.get(id), after.get(id));
        let (status, platform_ref, mut blocks) = match (old, new) {
            (Some(o), Some(n)) if o.prompt == n.prompt => continue,
            (Some(o), Some(n)) => ("changed", n.platform_ref.clone(), block_changes(&o.blocks, &n.blocks)),
            (None, Some(n)) => ("added", n.platform_ref.clone(), BTreeMap::new()),
            (Some(o), None) => ("removed", o.platform_ref.clone(), BTreeMap::new()),
            (None, None) => unreachable!(),
        };
        if status == "changed" && blocks.is_empty() {
            blocks.insert("workspace:sop.yaml".into(), "edited"); // e.g. a default variable changed
        }
        let diff = unified_diff(prompt_of(old), prompt_of(new), id);
        changes.push(AgentChange { agent: id.clone(), status, platform_ref, blocks, diff });
    }
    Plan { changes }
}

fn prompt_of(s: Option<&AgentSnapshot>) -> &str {
    s.map_or("", |s| s.prompt.as_str())
}

fn block_changes(old: &BTreeMap<String, String>, new: &BTreeMap<String, String>) -> BTreeMap<String, &'static str> {
    let mut out = BTreeMap::new();
    for (block, hash) in old {
        match new.get(block) {
            None => drop(out.insert(block.clone(), "removed")),
            Some(h) if h != hash => drop(out.insert(block.clone(), "edited")),
            _ => {}
        }
    }
    for block in new.keys().filter(|b| !old.contains_key(*b)) {
        out.insert(block.clone(), "added");
    }
    out
}

fn count(n: usize) -> String {
    if n == 1 {
        "1 agent".into()
    } else {
        format!("{n} agents")
    }
}

fn label(block: &str) -> String {
    let (kind, id) = block.split_once(':').unwrap_or(("", block));
    match kind {
        "workspace" => format!("`{id}`"),
        "sop" => format!("SOP `{id}`"),
        "agent" => format!("agent file `{id}`"),
        _ => format!("{kind} `{id}`"),
    }
}

fn verb(block: &str, change: &str) -> &'static str {
    if block.starts_with("agent:") || block.starts_with("workspace:") {
        return "edited";
    }
    match change {
        "added" => "added",
        "removed" => "removed",
        _ => "edited",
    }
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// (block, change) → agents, sorted by block then change.
    pub fn by_block(&self) -> BTreeMap<(String, &'static str), Vec<String>> {
        let mut groups: BTreeMap<(String, &'static str), Vec<String>> = BTreeMap::new();
        for c in &self.changes {
            for (block, change) in &c.blocks {
                groups.entry((block.clone(), change)).or_default().push(c.agent.clone());
            }
        }
        groups
    }

    /// One line per added or removed agent and per changed block.
    pub fn summary(&self) -> Vec<String> {
        let agents =
            self.changes.iter().filter(|c| c.status != "changed").map(|c| format!("agent `{}` {}", c.agent, c.status));
        let blocks = self.by_block().into_iter().map(|((block, change), ids)| {
            format!("{} {} → {}: {}", label(&block), verb(&block, change), count(ids.len()), ids.join(", "))
        });
        agents.chain(blocks).collect()
    }

    pub fn text(&self, diffs: bool) -> String {
        if self.is_empty() {
            return "No agent prompts change.\n".into();
        }
        let mut out = vec![format!("{} change:", count(self.changes.len()))];
        out.extend(self.summary().into_iter().map(|line| format!("  {line}")));
        if diffs {
            for c in &self.changes {
                out.extend([String::new(), c.diff.trim_end().to_string()]);
            }
        }
        out.join("\n") + "\n"
    }

    pub fn to_json(&self) -> Json {
        let changes: Vec<Json> = self.changes.iter().map(|c| {
            json!({"agent": c.agent, "platform_ref": c.platform_ref, "status": c.status, "blocks": c.blocks, "diff": c.diff})
        }).collect();
        let by_block: Vec<Json> = self
            .by_block()
            .into_iter()
            .map(|((block, change), agents)| json!({"block": block, "change": change, "agents": agents}))
            .collect();
        json!({"changes": changes, "by_block": by_block})
    }
}

// --- affected --------------------------------------------------------------------------------------

/// One agent to test.
pub struct AffectedAgent {
    pub id: String,
    pub platform_ref: String,
    pub platform: String,
    pub platform_id: String,
    pub reason: &'static str, // changed | added | requested | all
    pub changed: Vec<String>,
    pub changed_sops: Vec<String>,
    pub sops: Vec<String>,
}

impl AffectedAgent {
    fn new(head: &Build, id: &str, reason: &'static str) -> Self {
        let r = &head.agents[id];
        AffectedAgent {
            id: id.to_string(),
            platform_ref: r.agent.platform_ref(),
            platform: r.agent.platform().to_string(),
            platform_id: r.agent.platform_id().to_string(),
            reason,
            changed: vec![],
            changed_sops: vec![],
            sops: r.sops().map(|s| s.id.clone()).collect(),
        }
    }

    fn to_json(&self) -> Json {
        json!({
            "id": self.id, "platform_ref": self.platform_ref, "platform": self.platform,
            "platform_id": self.platform_id, "reason": self.reason, "changed": self.changed,
            "changed_sops": self.changed_sops, "sops": self.sops,
        })
    }
}

pub struct Affected {
    pub agents: Vec<AffectedAgent>,
    pub all: bool,
}

impl Affected {
    pub fn ids(&self) -> Vec<String> {
        self.agents.iter().map(|a| a.id.clone()).collect()
    }

    pub fn to_json(&self) -> Json {
        let agents: Vec<Json> = self.agents.iter().map(|a| a.to_json()).collect();
        json!({"all": self.all, "count": self.agents.len(), "agents": agents})
    }

    /// Values for $GITHUB_OUTPUT, in order.
    pub fn github_outputs(&self) -> Vec<(&'static str, String)> {
        let platform_ids: Vec<&str> = self.agents.iter().map(|a| a.platform_id.as_str()).collect();
        let matrix = Json::Array(self.agents.iter().map(|a| a.to_json()).collect());
        vec![
            ("ids", self.ids().join(" ")),
            ("platform_ids", platform_ids.join(" ")),
            ("matrix", ascii_json(&matrix.to_string())),
            ("count", self.agents.len().to_string()),
            ("all", self.all.to_string()),
        ]
    }

    /// The GitHub step summary; `compared` is the ref compared with, e.g. "origin/main (3f9a2c1)".
    pub fn markdown(&self, compared: Option<&str>) -> String {
        let compared = compared.map(|c| format!("Compared with `{c}`.\n\n")).unwrap_or_default();
        if self.agents.is_empty() {
            return format!("### Agents to test\n\n{compared}None.\n");
        }
        let why = if self.all { "all agents" } else { "affected by this change" };
        let heading = format!("### Agents to test ({}, {why})\n\n{compared}", self.agents.len());
        let mut lines = vec![heading.trim_end().to_string(), String::new()];
        for a in &self.agents {
            let detail =
                if a.changed.is_empty() { format!(" ({})", a.reason) } else { format!(": {}", a.changed.join(", ")) };
            lines.push(format!("- `{}` ({} `{}`){detail}", a.id, a.platform, a.platform_id));
        }
        lines.join("\n") + "\n"
    }
}

/// The agents to test. `requested` overrides the comparison; with a base, agents whose prompt
/// changed are picked; with `all_if_none` (or no base), every agent when nothing else is.
pub fn affected(
    head: &Build,
    base: Option<&Build>,
    requested: &[String],
    all_if_none: bool,
) -> Result<Affected, Issues> {
    if !requested.is_empty() {
        let ids = resolve_requested(head, requested)?;
        let agents = ids.iter().map(|id| AffectedAgent::new(head, id, "requested")).collect();
        return Ok(Affected { agents, all: false });
    }
    let mut picked = vec![];
    if let Some(base) = base {
        for c in make_plan(&snapshot(base), &snapshot(head)).changes {
            if c.status == "removed" {
                continue;
            }
            let mut a = AffectedAgent::new(head, &c.agent, if c.status == "added" { "added" } else { "changed" });
            a.changed = c.blocks.keys().cloned().collect();
            a.changed_sops = a.changed.iter().filter_map(|b| b.strip_prefix("sop:")).map(String::from).collect();
            picked.push(a);
        }
    }
    if !picked.is_empty() || (base.is_some() && !all_if_none) {
        return Ok(Affected { agents: picked, all: false });
    }
    let agents = head.agents.keys().map(|id| AffectedAgent::new(head, id, "all")).collect();
    Ok(Affected { agents, all: true })
}

/// Maps agent ids, platform refs and platform ids to agent ids.
pub fn resolve_requested(head: &Build, requested: &[String]) -> Result<Vec<String>, Issues> {
    let mut lookup: BTreeMap<String, String> = BTreeMap::new();
    for (id, r) in &head.agents {
        lookup.insert(r.agent.platform_ref(), id.clone());
        lookup.entry(r.agent.platform_id().to_string()).or_insert_with(|| id.clone());
    }
    for id in head.agents.keys() {
        lookup.insert(id.clone(), id.clone());
    }
    let (mut ids, mut unknown) = (Vec::<String>::new(), vec![]);
    for name in requested {
        match lookup.get(name) {
            None => unknown.push(name.as_str()),
            Some(id) if !ids.contains(id) => ids.push(id.clone()),
            _ => {}
        }
    }
    if !unknown.is_empty() {
        let known: Vec<&str> = head.agents.keys().map(String::as_str).collect();
        let msg = format!("unknown agent(s): {}. Known: {}", unknown.join(", "), known.join(", "));
        return Err(Issues(vec![Issue::error("unknown_agent", "", msg)]));
    }
    Ok(ids)
}
