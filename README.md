<div align="center">

# 📋🧩 sop

**dbt for agent instructions.**

The SOP compiler: modular, git-versioned instructions for teams managing multiple task-driven agents.

Write shared instructions and SOPs once. sop compiles each agent's full prompt<br>
and shows exactly which agents a change touches.

[Concepts](https://github.com/amanmibra/whatsop) · [Quickstart](#quickstart) · [How it works](#how-it-works) · [Usage](#usage) · [CLI reference](CLI.md) · [Format](FORMAT.md) · [Example](examples/livekit-restaurant)

</div>

---

## Why

An SOP is a standard operating procedure: what an agent should do in a given situation, step by step.

Running one agent per customer usually means dozens of near-identical prompts maintained by hand. A tone tweak means editing all of them. A new rule means remembering which ones need it. Copies drift.

sop keeps the shared parts in one place, in git, and compiles them into one prompt per agent.

### Why structure

With a free-text prompt, you can't say exactly what the agent was supposed to do, so you can't measure whether it did. Structure turns instructions into a spec you can check an agent against:

- **Mistakes have names.** Every step and never-do has a stable id, so call or chat QA can report "skipped step 2 of allergen-check" instead of "bad call," and track a pass rate per rule.
- **Changes are traceable.** `sop plan` shows which agents an edit touches before you merge. Compare pass rates before and after a release, and a regression points to a specific edit.
- **Shared parts are written once.** A tone tweak is one edit instead of thirty, and copies stop drifting.
- **Automated improvement can be reviewed.** An LLM rewriting a whole prompt produces a diff nobody can check. A proposed change to the one rule that keeps failing is a small diff a person can approve.
- **Conflicts surface early.** `sop lint` flags "always X" against "never X" before an agent hits it.
- **Non-engineers can own it.** Ops and support teams already write SOPs. The format reads like theirs.
- **There's an audit trail.** Git and rule ids show which version of which rule was live on any conversation.

This works for any task-driven agent, not only voice: support chat, ticket triage, back-office ops, email outreach. It starts paying off at around five agents, or when someone asks how often an agent gets a step wrong. One agent with one prompt doesn't need it.

Not related to Mozilla's `sops` (secrets).

## Quickstart

### Already have agent prompts?

Most teams start here: a prompt per agent, written by hand, mostly copied from each other. The `sop-import` skill converts them for you.

**1. Add the import skill to your repo** (one line, run in the repo root):

```sh
curl -fsSL https://raw.githubusercontent.com/sopops-dev/sop/main/skills/sop-import/SKILL.md --create-dirs -o .claude/skills/sop-import/SKILL.md
```

For Codex, use `-o .agents/skills/sop-import/SKILL.md` instead. OpenCode reads either.

**2. Run it in your coding agent:**

| Agent | Run |
|---|---|
| Claude Code | `/sop-import` |
| Codex | `$sop-import` (or pick it from `/skills`) |
| OpenCode | ask it to "use the sop-import skill" |

The skill installs the `sop` CLI if it's missing, finds your existing prompts (in code or files, or pulls them read-only from LiveKit, ElevenLabs, Vapi or Retell, helping you set up a key or MCP if needed), turns them into sop files, checks that nothing was lost, and asks you to approve a one-screen plan before it writes anything.

<details>
<summary>What the import does, step by step</summary>

| Step | Who | What happens |
|---|---|---|
| Collect | agent | Copies each existing prompt into `sops/originals/`, asks for anything it can't find |
| Map | `sop overlap` | Finds text every prompt shares, text some share, and near-copies that differ by a value (or have drifted) |
| Plan | you | Approve a one-screen plan: which instructions, SOPs and groups, how to handle drift |
| Build | agent | Writes the files, keeping the original wording |
| Verify | `sop compare` | Fails if any original sentence is missing or changed; the agent repeats until it passes |
| Review | `sop lint` | Flags duplicates and conflicts ("once" vs "twice" per call, "always X" vs "never X") for you to decide |
| Commit | you | Approve a one-screen summary before anything is committed |

</details>

For tools built on sop, `sop overlap --json` and `sop compare --json` print these results as JSON, with prompts read from a folder, a JSON file or stdin ([CLI.md](CLI.md#sop-overlap)).

<details>
<summary>Claude Code plugin (shares the skill with your whole team)</summary>

```
/plugin marketplace add sopops-dev/sop
/plugin install sop@sop
```

Then run `/sop:sop-import` (plugin skills are prefixed with the plugin name). To set it up for everyone who opens the repo, add the marketplace to your project's `.claude/settings.json` under `extraKnownMarketplaces` and enable `sop@sop` in `enabledPlugins`; see [Claude Code's plugin docs](https://code.claude.com/docs/en/plugins/marketplace-reference).

</details>

### Starting from scratch?

Install the CLI, copy [`examples/livekit-restaurant/sops`](examples/livekit-restaurant/sops) into your repo as `sops/`, and edit it. Every file is commented, and `sop guide` prints the full format reference. Coding agents can write sop files from it too.

```sh
curl -fsSL https://raw.githubusercontent.com/sopops-dev/sop/main/install.sh | sh
```

One binary for macOS, Linux and Windows. Version pinning, building from source and uninstalling are in the [CLI reference](CLI.md#install-and-uninstall).

### Then: load the compiled prompt in your agent

```ts
const instructions = readFileSync(`sops/build/${agentId}.prompt.md`, "utf8");
```

See the [LiveKit example](examples/livekit-restaurant) for a complete agent.

**Prefer a UI and a database to files in git?** [sopops](https://github.com/amanmibra/sopops) runs the same compiler behind forms: compose agents from blocks, review the diff, publish a release, and have each agent fetch its prompt when a call starts.

## How it works

<p align="center"><img src="docs/images/blocks.png" alt="Shared instructions, procedures and groups are written once as blocks; each agent lists the blocks it uses in order, after its own context; sop compiles one full prompt per agent for LiveKit, Vapi, ElevenLabs or Retell" width="860"></p>

Prompts are compiled from blocks, and each agent lists the blocks it uses:

| | File | Holds |
|---|---|---|
| 🧱 | `instructions/*.md` | shared text: identity, tone, context, policy |
| 📋 | `procedures/*.md` | SOPs: goal, steps, never-do's, warning signs, tools |
| 🎙️ | `agents/*.yaml` | one per agent: platform id, its own text, its blocks in order, values for `{{placeholders}}` |

A block doesn't say who uses it; the agent file does, in prompt order:

```yaml
livekit: tonys-pizza
context: |
  Tony's is a wood-fired pizza shop in Brooklyn. Pickup only after 10pm.
blocks:
  - restaurant-host
  - brand-voice
  - allergen-check
  - closing
variables:
  menu_allergen_link: tonys.com/allergens
```

An SOP reads like a checklist:

```markdown
# Allergen check

**Goal:** Customer leaves knowing whether their order is safe for their allergy.

## Steps
1. Ask if anyone in the order has a food allergy
2. Check each item against {{menu_allergen_link}} `tool: lookup_allergens` `required`

## Never
- Never say an item is "allergen-free" or "safe"
```

- **Groups** in `sop.yaml` name a set of blocks many agents share; an agent lists the group like a block.
- **Every rule has an id.** Each step, never and warning sign gets a stable id in `build/lock.json` and `sop export`, so call QA can name the exact rule an agent broke. Rewording a rule gives it a new id; reordering doesn't. See [lock.json](CLI.md#sop).
- **The format is checked strictly,** and every mistake is reported with its file and line. See the [SOP rules](FORMAT.md#sop-rules). SOPs can also be written in YAML; `sop convert` switches between the two.
- Coming from sopc v0.0.8 or earlier? `sop migrate` converts the folder and checks every prompt stays the same.

Change a shared file and see what moves before you merge:

```console
$ sop plan
Comparing with origin/main (3f9a2c1)
3 agents change:
  instruction `brand-voice` edited → 3 agents: luigis-trattoria, sakura-sushi, tonys-pizza
  SOP `reservations` edited → 2 agents: luigis-trattoria, sakura-sushi

--- a/sakura-sushi.prompt.md
+++ b/sakura-sushi.prompt.md
-Speak warmly and briefly. Ask one question at a time.
+Speak warmly and briefly. Use the caller's name once you have it. Ask one question at a time.
```

## Usage

Run from your repo. sop uses the `sops/` folder; add `--dir path/to/sops` to use another.

```sh
sop              # Compile every agent's prompt into sops/build
sop validate     # Find errors in the files
sop lint         # Find duplicated or conflicting instructions
sop fmt          # Format SOP files
sop plan         # Show which agents your changes affect, with prompt diffs
sop affected     # List the agents your changes affect (for CI)
sop agents       # List every agent with its platform id and SOPs
sop migrate      # Convert a folder from sopc v0.0.8 or earlier (shows the plan; --yes writes)
sop verify       # Check live prompts and tools on ElevenLabs, Vapi and Retell match the build
```

Every command and flag is in the [CLI reference](CLI.md), and `sop <command> --help` shows examples.

### In CI

```sh
sop --check        # Fail if sops/build is out of date
sop fmt --check    # Fail if SOP files aren't formatted
sop validate       # Fail on errors
sop lint --strict  # Fail on duplicated or conflicting instructions
```

To run your own tests on just the agents a pull request changes, see the [behavior gate](examples/behavior-gate). To block changes that make agents worse with Braintrust evals, see the [Braintrust example](examples/braintrust-evals). To catch prompts edited in a platform's dashboard, run `sop verify` nightly: see the [drift check](examples/drift-check).

<p align="center"><img src="docs/images/plan-and-evals.png" alt="sop plan showing a bad edit to a shared allergen SOP reaching 4 agents, and the CI eval check failing on the regressed calls with the transcript attached" width="760"></p>

<p align="center"><sub>A bad edit to one shared SOP: <code>sop plan</code> shows the 4 agents it reaches, and the <a href="examples/braintrust-evals">eval gate</a> blocks the PR with the failing call.</sub></p>

## Working with coding agents

[FORMAT.md](FORMAT.md) is the full format reference, written for people and agents, and `sop guide` prints it. Add this to your project's `AGENTS.md` (Codex, OpenCode) or `CLAUDE.md` (Claude Code):

```markdown
Agent instructions live in `sops/` in the sop format.
Run `sop guide` and read it before editing anything there.
Finish with `sop fmt`, `sop validate`, `sop lint` and `sop plan`.
Never pass `--yes` to `sop fmt`, `sop convert` or `sop migrate` unless I've approved what it lists.
```

For editor autocomplete, point `yaml-language-server` at the schemas in [`spec/`](spec).

## Status

Early. sop is the format and the tools to compile and check it; serving prompts and evaluating calls are left to other tools. See the [roadmap](ROADMAP.md).

## Contributing

Contributions welcome. Open an issue first for anything big. See [CONTRIBUTORS.md](CONTRIBUTORS.md).

```sh
cargo test
```

## License

[Apache-2.0](LICENSE)
