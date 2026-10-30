export type TerminalOutputQuietOutcome = 'prompt' | 'quiet' | 'timeout' | 'unavailable'

// Footer/hint text Claude Code and Codex render under their input box once
// it's live. Compared with ANSI codes and all whitespace removed, since TUIs
// often position text with cursor moves instead of literal spaces.
const CLI_PROMPT_MARKERS = [
  '? for shortcuts',
  'bypass permissions on',
  'accept edits on',
  'plan mode on',
  'context left',
].map((marker) => marker.replace(/\s+/g, '').toLowerCase())

function stripAnsiSequences(text: string): string {
  return text.replace(
    // eslint-disable-next-line no-control-regex
    /\u001b(?:\][^\u0007]*(?:\u0007|\u001b\\)|\[[0-?]*[ -/]*[@-~]|[@-Z\\-_])/g,
    '',
  )
}

export function hasCliPromptMarker(terminalText: string): boolean {
  const compact = stripAnsiSequences(terminalText).replace(/\s+/g, '').toLowerCase()
  return CLI_PROMPT_MARKERS.some((marker) => compact.includes(marker))
}

/** Startup automation must see an input prompt, not just a persistent footer. */
export function hasIdleCliPrompt(terminalText: string): boolean {
  const text = stripAnsiSequences(terminalText)
  return hasCliPromptMarker(text) &&
    /^\s*[›❯>]\s*/m.test(text) &&
    !/esc\s+to\s+(?:interrupt|cancel|stop)|connecting\s+to|starting\s+mcp|trust\s+(?:this|the)\s+(?:folder|directory)/i.test(text)
}

export interface TerminalOutputProbe {
  /** Output sequence number of the session (counts output frames). */
  seq: number
  /** Whether the CLI's own input prompt is recognisably on screen. */
  promptVisible: boolean
}

export interface WaitForTerminalOutputQuietOptions {
  /** Current state of the session, or null once it's gone. */
  probe: () => Promise<TerminalOutputProbe | null>
  sleep: (ms: number) => Promise<void>
  now: () => number
  /** Output must stay unchanged this long after the prompt shows up. */
  promptSettleMs: number
  /** Fallback (no prompt marker): never report quiet before this much time. */
  minWaitMs: number
  /** Fallback: how long the output must stay unchanged to count as quiet. */
  quietMs: number
  /**
   * Fallback: output frames required beyond the first probe before quiet can
   * count. The CLI is launched by typing its command into a shell, so the
   * first few frames are the shell's prompt/echo, not the CLI — a quiet gap
   * right after those is the CLI still booting, which is exactly when typed
   * input gets lost.
   */
  minFramesForQuiet: number
  /** Give up waiting (and let the caller proceed anyway) after this long. */
  maxWaitMs: number
  pollMs: number
  /** Disable the heuristic quiet fallback for automatic prompt injection. */
  requirePrompt?: boolean
}

/**
 * Resolves once a freshly launched CLI looks ready to take typed input.
 * Preferred signal: the CLI's prompt marker is on screen and output has
 * briefly settled. Fallback, for CLIs whose prompt isn't recognised: enough
 * output has been produced to rule out just the shell, and it then went
 * quiet. Neither can tell a startup dialog (e.g. a trust prompt) from the
 * real prompt if the dialog happens to match — the fallback can't at all.
 */
export async function waitForTerminalOutputQuiet({
  probe,
  sleep,
  now,
  promptSettleMs,
  minWaitMs,
  quietMs,
  minFramesForQuiet,
  maxWaitMs,
  pollMs,
  requirePrompt = false,
}: WaitForTerminalOutputQuietOptions): Promise<TerminalOutputQuietOutcome> {
  const startedAt = now()
  const first = await probe()
  if (first === null) {
    return 'unavailable'
  }
  const baselineSeq = first.seq
  let lastSeq = first.seq
  let lastChangeAt = startedAt

  for (;;) {
    await sleep(pollMs)
    const state = await probe()
    if (state === null) {
      return 'unavailable'
    }
    const at = now()
    if (state.seq !== lastSeq) {
      lastSeq = state.seq
      lastChangeAt = at
    }
    const quietFor = at - lastChangeAt
    if (state.promptVisible && quietFor >= promptSettleMs) {
      return 'prompt'
    }
    if (
      !requirePrompt &&
      lastSeq - baselineSeq >= minFramesForQuiet &&
      at - startedAt >= minWaitMs &&
      quietFor >= quietMs
    ) {
      return 'quiet'
    }
    if (at - startedAt >= maxWaitMs) {
      return 'timeout'
    }
  }
}
