const { execFile } = require('child_process');

const SUMMARY_HISTORY_CAP = 10;

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
    const reader = session.getContextReader();

    if (reader) {
      // Context-aware path: read structured agent data
      await handleContextAware(session, reader, state);
    } else {
      // Fallback: terminal output (plain shell, no agent detected)
      await handleTerminalFallback(session, state);
    }
  }
}

async function handleContextAware(session, reader, state) {
  let result;
  try {
    result = reader.read();
  } catch (e) {
    console.error(`[thought] ${session.id}: context reader error:`, e.message);
    return;
  }

  if (!result) {
    console.log(`[thought] ${session.id}: skip (context unchanged)`);
    return;
  }

  const { snapshot } = result;
  const prompt = buildContextPrompt(snapshot, state, session._summaryHistory);

  console.log(`[thought] ${session.id}: calling codex (context-aware, state=${state}, task="${(snapshot.userTask || '').slice(0, 50)}")`);

  try {
    const thought = await callCodex(prompt);
    console.log(`[thought] ${session.id}: codex returned: "${thought}"`);
    if (thought) {
      session.thought = thought;
      session._summaryHistory.push(thought);
      if (session._summaryHistory.length > SUMMARY_HISTORY_CAP) {
        session._summaryHistory = session._summaryHistory.slice(-SUMMARY_HISTORY_CAP);
      }
      broadcastThought(session);
    }
  } catch (e) {
    console.error(`[thought] ${session.id}: codex error:`, e.message);
  }
}

async function handleTerminalFallback(session, state) {
  const hash = session.replayHash();
  if (hash === session._lastReplayHash) {
    console.log(`[thought] ${session.id}: skip (unchanged hash)`);
    return;
  }
  session._lastReplayHash = hash;

  const context = session.getThoughtContext();
  if (!context.trim()) {
    console.log(`[thought] ${session.id}: skip (empty context)`);
    return;
  }

  const prevContext = session._lastThoughtContext;
  session._lastThoughtContext = context;

  const prompt = buildTerminalPrompt(context, state, prevContext);

  console.log(`[thought] ${session.id}: calling codex (terminal-fallback, state=${state}, context=${context.length} chars)`);

  try {
    const thought = await callCodex(prompt);
    console.log(`[thought] ${session.id}: codex returned: "${thought}"`);
    if (thought) {
      session.thought = thought;
      session._summaryHistory.push(thought);
      if (session._summaryHistory.length > SUMMARY_HISTORY_CAP) {
        session._summaryHistory = session._summaryHistory.slice(-SUMMARY_HISTORY_CAP);
      }
      broadcastThought(session);
    }
  } catch (e) {
    console.error(`[thought] ${session.id}: codex error:`, e.message);
  }
}

function buildContextPrompt(snapshot, state, summaryHistory) {
  const parts = [];
  parts.push('You are observing a coding agent\'s work session.');
  parts.push(`Agent state: ${state}`);
  parts.push('');

  if (snapshot.userTask) {
    parts.push('User\'s request:');
    parts.push(`"${snapshot.userTask}"`);
    parts.push('');
  }

  if (summaryHistory.length > 0) {
    const recent = summaryHistory.slice(-5);
    parts.push('Previous observations (oldest to newest):');
    for (const s of recent) {
      parts.push(`- ${s}`);
    }
    parts.push('');
  }

  if (snapshot.recentActions && snapshot.recentActions.length > 0) {
    parts.push('Recent agent actions:');
    for (const a of snapshot.recentActions) {
      if (a.tool === 'said') {
        parts.push(`- Said: "${a.detail}"`);
      } else {
        parts.push(`- Used tool: ${a.tool}${a.detail ? ` (${a.detail})` : ''}`);
      }
    }
    parts.push('');
  }

  if (snapshot.currentTool) {
    const ct = snapshot.currentTool;
    parts.push(`Right now: ${ct.tool}${ct.detail ? ` — ${ct.detail}` : ''}`);
    parts.push('');
  }

  parts.push('---');
  parts.push('Summarize what the agent is working on RIGHT NOW in 3-8 words.');
  parts.push('Be specific about the task, not the tool. No quotes, no preamble.');
  parts.push('Examples: "fixing auth token refresh", "reading test files for context", "writing new API endpoint"');

  return parts.join('\n');
}

function buildTerminalPrompt(context, state, prevContext) {
  const isFirst = !prevContext;
  let contextBlock;
  if (isFirst) {
    contextBlock = `Full terminal output:\n${context}`;
  } else {
    const overlap = context.indexOf(prevContext.slice(-200));
    const delta = overlap >= 0 ? context.slice(overlap + prevContext.slice(-200).length) : context;
    contextBlock = delta.trim()
      ? `New output since last check:\n${delta}`
      : `Terminal output (unchanged):\n${context.slice(-200)}`;
  }

  return `You are monitoring a terminal session.
State: ${state}
${contextBlock}
---
Identify what TASK is happening right now. Respond with ONLY 3-8 words. No quotes, no preamble.
Focus on the task/goal, not the tool or command.
Examples: "fixing auth token refresh", "adding dark mode toggle", "debugging failing test suite", "waiting for user input", "idle at shell prompt"`;
}

function callCodex(prompt) {
  return new Promise((resolve, reject) => {
    execFile(
      'codex',
      ['-m', 'codex-mini-latest', 'exec', '-c', 'model_reasoning_effort="low"', '--ephemeral', prompt],
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
