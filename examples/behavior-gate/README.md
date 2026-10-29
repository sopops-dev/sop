# 📋🧩 Example: a behavior gate for any test setup

A GitHub Action that works out **which agents a pull request changes** and hands their ids to **your own test command**. Use it with whatever you already have: Braintrust, pytest, a simulation tool, a script that calls your agents.

```
.github/workflows/behavior-gate.yml   copy into your repo, then fill in one step
```

## What it does

| Situation | Agents tested |
|---|---|
| PR changes SOP files | The agents whose built prompt changed, compared with the base branch |
| PR changes only tests, or SOP edits that don't change any prompt | All agents |
| Started by hand with `agents: "la-casita asst_9f3e"` | Those agents, by agent id or platform id (unknown ids fail the run) |
| Started by hand with no agents | All agents |

Agents removed in the PR are skipped. A shared change reaches only the agents that use it: editing the English brand voice in the [restaurant example](../livekit-restaurant) tests three agents and skips `la-casita`, which uses the Spanish one.

## What your command gets

The detect job is one command, `sop affected --against origin/<base branch> --all-if-none --ci`, and the `test` job passes its results to your command:

| Variable | Example |
|---|---|
| `AGENT_IDS` | `tonys-pizza sakura-sushi` (agent ids: the file names in `agents/`) |
| `AGENT_PLATFORM_IDS` | `tonys-pizza asst_9f3e` (each agent's own id on its platform: LiveKit `agent_name`, Vapi assistant id, ElevenLabs `agent_id`) |
| `AGENTS_JSON` | one object per agent (main fields below) |
| `ALL_AGENTS` | `true` when testing every agent |

```json
{
  "id": "tonys-pizza",
  "platform": "livekit",
  "platform_id": "tonys-pizza",
  "reason": "changed",
  "changed": ["sop:allergen-check"],
  "changed_sops": ["allergen-check"],
  "sops": ["allergen-check", "delivery-handling", "large-orders"]
}
```

`changed_sops` lets a test suite go one level finer: run only the cases for the SOPs that changed, not every case for the agent.

Fill in the one step:

```yaml
- name: Run your tests
  run: npm run test:agents -- --agents "$AGENT_IDS"
```

Other shapes it fits:

```sh
pytest tests/agents -k "$(echo $AGENT_IDS | sed 's/ / or /g')"     # pytest, filtered by agent
for id in $AGENT_PLATFORM_IDS; do ./scripts/simulate.sh "$id"; done  # one run per agent
python evals/run_evals.py --base origin/main                        # the Braintrust example
```

To test agents in parallel instead, uncomment the `test-each` job: it runs once per agent with `matrix.agent.id`, `matrix.agent.platform_id` and so on.

## Set it up

1. Copy `.github/workflows/behavior-gate.yml` into your repo.
2. If your sop folder isn't `sops/` (or the repo root), set `SOP_ROOT` to it and change the `sops/**` path. Add the `paths:` your tests live in.
3. Replace the placeholder in **Run your tests** with your command (it fails on purpose until you do).
4. Make the **behavior-gate / test** check required in your branch protection rules, so failing tests block the merge.

The same command works outside GitHub Actions:

```sh
sop affected                                   # agent ids, one per line, compared with your default branch
sop affected --format platform-ids             # the platforms' own ids
sop affected --format json                     # everything, including changed SOPs
sop affected --against release                 # compared with another branch or ref
sop affected --agents "asst_9f3e la-casita"    # specific agents (either kind of id)
```
