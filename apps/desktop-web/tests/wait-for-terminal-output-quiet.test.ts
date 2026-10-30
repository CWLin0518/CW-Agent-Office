import test from 'node:test'
import assert from 'node:assert/strict'
import {
  hasCliPromptMarker,
  hasIdleCliPrompt,
  waitForTerminalOutputQuiet,
  type TerminalOutputProbe,
} from '../src/features/terminal/wait-for-terminal-output-quiet.js'

const TIMING = {
  promptSettleMs: 500,
  minWaitMs: 2000,
  quietMs: 3000,
  minFramesForQuiet: 5,
  maxWaitMs: 30000,
  pollMs: 250,
}

// Drives a fake clock: each sleep advances time, and `probeAt(t)` says what
// the session looks like at time t (null = session gone).
function run(probeAt: (t: number) => TerminalOutputProbe | null, requirePrompt = false) {
  let clock = 0
  return waitForTerminalOutputQuiet({
    ...TIMING,
    requirePrompt,
    probe: async () => probeAt(clock),
    sleep: async (ms) => {
      clock += ms
    },
    now: () => clock,
  }).then((outcome) => ({ outcome, elapsedMs: clock }))
}

const noPrompt = (seq: number): TerminalOutputProbe => ({ seq, promptVisible: false })

test('shell echo followed by a silent CLI boot is not mistaken for ready', async () => {
  // Shell prompt/echo frames at 0–300ms, CLI silent until 5s, prints until 6s,
  // prompt marker visible from 6s on.
  const result = await run((t) => {
    if (t < 300) return noPrompt(Math.floor(t / 100))
    if (t < 5000) return noPrompt(3)
    if (t < 6000) return noPrompt(3 + Math.floor((t - 5000) / 100))
    return { seq: 13, promptVisible: true }
  })
  assert.equal(result.outcome, 'prompt')
  assert.equal(result.elapsedMs, 6500)
})

test('automatic resume never treats a quiet startup dialog as ready', async () => {
  const result = await run((t) => noPrompt(t < 6000 ? Math.floor(t / 100) : 60), true)
  assert.equal(result.outcome, 'timeout')
})

test('automatic resume waits for an actual prompt after a long startup', async () => {
  const result = await run((t) => ({ seq: t < 20000 ? 60 : 61, promptVisible: t >= 20000 }), true)
  assert.equal(result.outcome, 'prompt')
  assert.equal(result.elapsedMs, 20500)
})

test('idle prompt requires input marker and rejects working and startup screens', () => {
  assert.equal(hasIdleCliPrompt('› Try a task\n100% context left'), true)
  assert.equal(hasIdleCliPrompt('❯ \n? for shortcuts'), true)
  assert.equal(hasIdleCliPrompt('100% context left'), false)
  assert.equal(hasIdleCliPrompt('› \nWorking (esc to interrupt)\n100% context left'), false)
  assert.equal(hasIdleCliPrompt('› \nStarting MCP servers\n100% context left'), false)
})

test('falls back to quiet when the prompt marker is never recognised', async () => {
  // Plenty of CLI output until 6s, then idle; no marker.
  const result = await run((t) => noPrompt(t < 6000 ? Math.floor(t / 100) : 60))
  assert.equal(result.outcome, 'quiet')
  assert.equal(result.elapsedMs, 9000)
})

test('a few shell frames alone never satisfy the quiet fallback', async () => {
  const result = await run((t) => noPrompt(t < 300 ? Math.floor(t / 100) : 3))
  assert.equal(result.outcome, 'timeout')
  assert.equal(result.elapsedMs, 30000)
})

test('times out when output never settles', async () => {
  const result = await run((t) => noPrompt(t))
  assert.equal(result.outcome, 'timeout')
})

test('reports unavailable when the session is gone from the start', async () => {
  const result = await run(() => null)
  assert.equal(result.outcome, 'unavailable')
  assert.equal(result.elapsedMs, 0)
})

test('reports unavailable once the session disappears', async () => {
  const result = await run((t) => (t < 1000 ? noPrompt(t) : null))
  assert.equal(result.outcome, 'unavailable')
  assert.equal(result.elapsedMs, 1000)
})

test('hasCliPromptMarker sees through ANSI codes and cursor-move spacing', () => {
  assert.equal(hasCliPromptMarker('\u001b[2m?\u001b[1Cfor\u001b[1Cshortcuts\u001b[22m'), true)
  assert.equal(hasCliPromptMarker('  \u001b[31m⏵⏵ bypass permissions on\u001b[0m (shift+tab to cycle)'), true)
  assert.equal(hasCliPromptMarker('PS C:\\work> claude --dangerously-skip-permissions'), false)
})
