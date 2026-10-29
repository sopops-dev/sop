# Roadmap

sop is the format for writing, versioning and building agent instructions, plus the tools to check them. It stays small on purpose. Serving prompts, evaluating calls and editing UIs belong in other tools that read the format.

## Done

- **Format.** Two kinds of self-contained block, instructions (shared prompt text) and SOPs (goal, scope, guidance, steps, forbidden actions, warning signs, tools), and agents keyed by platform id that list the blocks they use, in order. Groups for sets of blocks many agents share. JSON Schemas in `spec/`; reference in [FORMAT.md](FORMAT.md).
- **`migrate`.** Converts folders from the earlier format, where blocks chose their agents, and checks every prompt stays the same.
- **Build and validate.** One full prompt per agent, `lock.json` with block hashes and referenced tools, clear errors (including YAML colon traps).
- **`plan`.** Which agents a change touches and because of which block, with a prompt diff per agent.
- **`lint`.** Duplicated text, number conflicts, "always X" vs "never X", unused variables; `--strict` to fail CI on any of them.
- **Import.** The `/sop-import` skill for coding agents, with `overlap` and `compare` to prove nothing was lost.
- **`verify`.** Compares each agent's live prompt on ElevenLabs, Vapi or Retell with the compiled one, to catch dashboard hotfixes that git doesn't have ([drift check](examples/drift-check)).
- **LiveKit example.** A TypeScript agent that loads its prompt from the build, with mock tools ([examples/livekit-restaurant](examples/livekit-restaurant)).

## Next

1. **Tool check.** Warn when an SOP names a tool the agent doesn't register.
2. **More checks.** Vague or uncheckable rules, prompt length per agent. A semantic conflict check (an LLM reading each agent's prompt) could catch contradictions `lint` misses because they're worded differently ("never upsell more than once" vs "upsell up to 2 times").

3. **`sop test`.** Test cases kept in the repo (caller turns, mock tool results, what a good agent does), run against just the agents a change affects, the way `dbt test` runs on the models a change touches. The [behavior gate](examples/behavior-gate) and [Braintrust example](examples/braintrust-evals) are the manual version today; grading itself would come from sopqa.
4. **Locks, with the suggestion pipeline.** Locks (`locked: true`, removed for now) come back when there is something for them to do: once QA on real calls and feedback from users and builders turn into suggested instruction edits, a lock marks a block as off-limits to those automated edits.
5. **Hotfix backfill.** When `verify` finds drift, open a pull request that writes the live prompt's change back into the right block, so a dashboard hotfix becomes a reviewed change instead of being reverted by the next deploy.

Beyond that, the next steps come from teams using it.

## Out of scope (other tools read the format)

- **Serving prompts at call start, publishing on merge:** sopserve, a separate project.
- **Evaluating calls against SOPs and suggesting changes:** sopqa (planned) or any QA tool. `lock.json` tells them exactly which SOP version each call ran with. Marking blocks they must not suggest changes to is planned (see Locks above).
- **Editors and UIs.**

## Principles

- Git holds what's accepted. Nothing reaches an agent without going through it.
- An agent's integration is one call: load the prompt.
- Platform ids, not new ids: agents are identified by what the platform already calls them.
