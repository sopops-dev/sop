import { type JobContext, ServerOptions, cli, defineAgent, inference, voice } from '@livekit/agents';
import dotenv from 'dotenv';
import { fileURLToPath } from 'node:url';
import { loadInstructions } from './instructions.js';
import { mockTools } from './tools.js';

dotenv.config({ path: '.env.local' });

// One LiveKit agent per restaurant. Each deployment sets AGENT_NAME
// (tonys-pizza, luigis-trattoria, sakura-sushi), matching `livekit:` in sops/agents/.
const agentName = process.env.AGENT_NAME;
if (!agentName) throw new Error('Set AGENT_NAME, e.g. AGENT_NAME=tonys-pizza');

// Read once at startup. The prompt only changes when a new build is deployed.
const instructions = loadInstructions(agentName);
console.log(`sop: ${instructions.id} prompt ${instructions.hash.slice(0, 12)}`);

export default defineAgent({
  entry: async (ctx: JobContext) => {
    const session = new voice.AgentSession({
      // Use whatever STT / LLM / TTS you already run; sop only supplies `instructions`.
      stt: new inference.STT({ model: 'assemblyai/universal-3-5-pro', language: 'en' }),
      llm: new inference.LLM({ model: 'google/gemma-4-31b-it' }),
      tts: new inference.TTS({ model: 'fishaudio/s2.1-pro', voice: 'fa4c9eb3dccc4806b382b40d61c6b10a' }),
      turnHandling: { turnDetection: new inference.TurnDetector() },
    });

    await session.start({
      // The SOPs tell the agent to use these tools by name, so the agent must register them.
      agent: voice.Agent.create({ instructions: instructions.text, tools: mockTools }),
      room: ctx.room,
    });
    await ctx.connect();
    session.generateReply({ instructions: 'Greet the caller and ask how you can help.' });
  },
});

cli.runApp(new ServerOptions({ agent: fileURLToPath(import.meta.url), agentName }));
