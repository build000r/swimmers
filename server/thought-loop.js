const { execFile } = require('child_process');

function startThoughtLoop(sessionManager) {
  setInterval(() => generateThoughts(sessionManager), 15000);
  console.log('  thought generation loop started');
}

async function generateThoughts(sessionManager) {
  const sessionCount = sessionManager.sessions.size;
  console.log(`[thought] tick — ${sessionCount} sessions`);

  for (const session of sessionManager.sessions.values()) {
    if (session._exited) {
      console.log(`[thought] ${session.id}: skip (exited)`);
      continue;
    }

    const state = session.detector.state;

    const hash = session.replayHash();
    if (hash === session._lastReplayHash) {
      console.log(`[thought] ${session.id}: skip (unchanged hash)`);
      continue;
    }
    session._lastReplayHash = hash;

    const context = session.getThoughtContext();
    if (!context.trim()) {
      console.log(`[thought] ${session.id}: skip (empty context)`);
      continue;
    }

    const prevContext = session._lastThoughtContext;
    session._lastThoughtContext = context;

    console.log(`[thought] ${session.id}: calling codex (state=${state}, first=${!prevContext}, context=${context.length} chars)`);

    try {
      const thought = await callCodex(context, state, prevContext);
      console.log(`[thought] ${session.id}: codex returned: "${thought}"`);
      if (thought) {
        session.thought = thought;
        broadcastThought(session);
      }
    } catch (e) {
      console.error(`[thought] ${session.id}: codex error:`, e.message);
    }
  }
}

function callCodex(context, state, prevContext) {
  const isFirst = !prevContext;
  let contextBlock;
  if (isFirst) {
    contextBlock = `Full terminal output:\n${context}`;
  } else {
    // Find the new content since last check
    const overlap = context.indexOf(prevContext.slice(-200));
    const delta = overlap >= 0 ? context.slice(overlap + prevContext.slice(-200).length) : context;
    contextBlock = delta.trim()
      ? `New output since last check:\n${delta}`
      : `Terminal output (unchanged):\n${context.slice(-200)}`;
  }

  const prompt = `You are monitoring a coding agent's terminal session.
State: ${state}
${contextBlock}
---
Identify what TASK the agent is working on right now. Respond with ONLY 3-8 words. No quotes, no preamble.
Focus on the task/goal, not the tool or command.
Examples: "fixing auth token refresh", "adding dark mode toggle", "debugging failing test suite", "waiting for user input", "idle at shell prompt"`;

  return new Promise((resolve, reject) => {
    execFile(
      'codex',
      ['-m', 'gpt-5.3-codex-spark', 'exec', '-c', 'model_reasoning_effort="low"', '--ephemeral', prompt],
      { timeout: 15000 },
      (err, stdout, stderr) => {
        if (err) {
          console.error(`[thought] execFile failed: ${err.message}`);
          if (stderr) console.error(`[thought] stderr: ${stderr.slice(0, 200)}`);
          return reject(err);
        }
        resolve(stdout.trim());
      }
    );
  });
}

function broadcastThought(session) {
  const payload = JSON.stringify({ sessionId: session.id, thought: session.thought });
  const frame = Buffer.concat([Buffer.from([0x05]), Buffer.from(payload)]);

  const hasWs = !!(session.attachedWs && session.attachedWs.readyState === 1);
  console.log(`[thought] ${session.id}: broadcasting "${session.thought}" (ws attached: ${hasWs})`);

  if (hasWs) {
    session.attachedWs.send(frame);
  }
}

module.exports = { startThoughtLoop };
