// Loads this agent's instructions from the prompts sop built into sops/build/.
//
// sop compiles one full prompt per agent (`sop`). The files are
// committed and copied into the Docker image, so the agent reads them from disk:
// no network call, nothing that can be down. A prompt change ships with the next
// `lk agent deploy`.

import { readFileSync } from 'node:fs';
import { join } from 'node:path';

const BUILD_DIR = join(process.cwd(), 'sops', 'build');

interface Lock {
  agents: Record<string, { platform_ref: string; hash: string }>;
}

export interface Instructions {
  /** The sop agent id, e.g. "tonys-pizza". */
  id: string;
  /** The full prompt to pass to voice.Agent. */
  text: string;
  /** Hash of this exact prompt, from lock.json. Log it to tie calls to a version. */
  hash: string;
}

/** Instructions for the LiveKit agent registered under `agentName`. */
export function loadInstructions(agentName: string): Instructions {
  const lock: Lock = JSON.parse(readFileSync(join(BUILD_DIR, 'lock.json'), 'utf8'));
  const entry = Object.entries(lock.agents).find(([, a]) => a.platform_ref === `livekit:${agentName}`);
  if (!entry) {
    throw new Error(`No sop agent with "livekit: ${agentName}". Add sops/agents/<id>.yaml and run \`sop\`.`);
  }
  const [id, { hash }] = entry;
  return { id, hash, text: readFileSync(join(BUILD_DIR, `${id}.prompt.md`), 'utf8') };
}
