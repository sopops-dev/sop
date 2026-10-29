#!/usr/bin/env python3
"""Generates a large sop workspace in a git repo, for benchmarking `sop`, `lint`, `plan` and
`affected`.

    python3 scripts/bench-workspace.py /tmp/bench [--agents 300] [--instructions 200]
        [--procedures 100] [--commits 5]

Writes DIR/sops (a git repo at DIR with one commit per edit round; HEAD~N is the starting
point) and prints the commands to time. Output is deterministic for the same arguments.
"""

import argparse
import os
import random
import subprocess

WORDS = (
    "customer order table menu item delivery address payment card refund allergy guest booking time "
    "date party size manager staff kitchen chef special drink dessert discount coupon wait list phone "
    "name number email receipt pickup counter catering event tip note request change cancel confirm "
    "check ask tell offer repeat explain transfer hold call note record verify update"
).split()
OPENERS = ["Always", "Never", "Do not", "Please", "Make sure to", "Remember to", "Only", "Politely"]


def sentence(rng, numbers=True):
    n = rng.randint(6, 16)
    words = [rng.choice(WORDS) for _ in range(n)]
    if numbers and rng.random() < 0.2:
        words.insert(rng.randint(0, n), str(rng.randint(1, 60)))
    return f"{rng.choice(OPENERS)} {' '.join(words)}."


def variant(rng, text):
    """A near-copy: one word swapped, or a number changed, or a negation added."""
    words = text.rstrip(".").split(" ")
    r = rng.random()
    if r < 0.4:
        words[rng.randrange(len(words))] = rng.choice(WORDS)
    elif r < 0.7:
        words.append(str(rng.randint(61, 99)))
    else:
        words.insert(1, "never")
    return " ".join(words) + "."


def main():
    p = argparse.ArgumentParser()
    p.add_argument("dir")
    p.add_argument("--agents", type=int, default=300)
    p.add_argument("--instructions", type=int, default=200)
    p.add_argument("--procedures", type=int, default=100)
    p.add_argument("--blocks", type=int, default=40, help="instructions per agent")
    p.add_argument("--sops", type=int, default=20, help="procedures per agent")
    p.add_argument("--commits", type=int, default=5)
    a = p.parse_args()
    rng = random.Random(42)
    root = os.path.join(a.dir, "sops")
    for sub in ("instructions", "procedures", "agents"):
        os.makedirs(os.path.join(root, sub), exist_ok=True)

    def write(rel, text):
        with open(os.path.join(root, rel), "w") as f:
            f.write(text)

    write("sop.yaml", "version: 1\nvariables:\n  staff_transfer: the manager on duty\n")
    pool = []  # sentences, so some blocks repeat or contradict others
    for i in range(a.instructions):
        lines = []
        for _ in range(rng.randint(3, 6)):
            if pool and rng.random() < 0.08:
                lines.append(variant(rng, rng.choice(pool)))
            else:
                s = sentence(rng)
                pool.append(s)
                lines.append(s)
        write(f"instructions/inst-{i:03}.md", " ".join(lines) + "\n")
    for i in range(a.procedures):
        steps = "\n".join(f"{n + 1}. {sentence(rng).rstrip('.')}" for n in range(rng.randint(3, 7)))
        never = "\n".join(f"- {sentence(rng).rstrip('.')}" for _ in range(rng.randint(1, 3)))
        body = (
            f"# Procedure {i}\n\n**Goal:** {sentence(rng, False)}\n**When:** {sentence(rng, False)}\n\n"
            f"## Steps\n{steps}\n\n## Never\n{never}\n"
        )
        write(f"procedures/proc-{i:03}.md", body)
    inst = [f"inst-{i:03}" for i in range(a.instructions)]
    procs = [f"proc-{i:03}" for i in range(a.procedures)]
    for i in range(a.agents):
        blocks = rng.sample(inst, min(a.blocks, len(inst))) + rng.sample(procs, min(a.sops, len(procs)))
        listed = "\n".join(f"  - {b}" for b in blocks)
        write(
            f"agents/agent-{i:03}.yaml",
            f"livekit: agent-{i:03}\ncontext: |\n  {sentence(rng)}\nblocks:\n{listed}\n",
        )

    def git(*args):
        subprocess.run(["git", *args], cwd=a.dir, check=True, stdout=subprocess.DEVNULL)

    git("init", "-q")
    git("add", "-A")
    git("-c", "user.name=bench", "-c", "user.email=bench@example.com", "commit", "-qm", "start")
    for c in range(a.commits):
        for rel in rng.sample([f"instructions/{b}.md" for b in inst], 3):
            with open(os.path.join(root, rel), "a") as f:
                f.write(sentence(rng) + "\n")
        git("add", "-A")
        git("-c", "user.name=bench", "-c", "user.email=bench@example.com", "commit", "-qm", f"edit {c}")
    print(f"wrote {root}; try:")
    print(f"  sop --dir {root} -o {a.dir}/out")
    print(f"  sop lint --dir {root}")
    print(f"  sop plan --summary --dir {root} --against HEAD~{a.commits}")
    print(f"  sop affected --dir {root} --against HEAD~{a.commits}")


if __name__ == "__main__":
    main()
