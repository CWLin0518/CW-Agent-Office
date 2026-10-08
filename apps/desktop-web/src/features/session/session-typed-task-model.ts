// Rebuilds the line a user is typing into a station terminal from raw xterm
// input, so the session can be named after the first prompt they submit.
// Approximate by design: it only needs to recover readable task text.

const TYPED_TASK_BUFFER_MAX_CHARS = 2000
// Terminal replies arrive on the same channel as keystrokes (e.g. the answer to
// a CLI's colour query, `ESC ] 10 ; rgb:e6e6/eded/f7f7 BEL`), so string
// sequences (OSC/DCS/APC/PM/SOS) are removed whole — terminated by BEL or
// ST, or to the end of the chunk when unterminated — before CSI (incl.
// bracketed-paste markers, arrow keys, DA/DSR replies), SS3 and two-char escapes.
const ESCAPE_SEQUENCE_PATTERN =
  // eslint-disable-next-line no-control-regex
  /\x1b[\]P_^X][\s\S]*?(?:\x07|\x1b\\|$)|\x1b\[[0-9;?<=>]*[ -/]*[@-~]|\x1bO.|\x1b./g

export function appendTypedTaskInput(buffer: string, data: string): string {
  let next = buffer
  for (const char of data.replace(ESCAPE_SEQUENCE_PATTERN, '')) {
    if (char === '\x7f' || char === '\b') {
      next = Array.from(next).slice(0, -1).join('')
    } else if (char === '\x15' || char === '\x03') {
      // Ctrl+U clears the line, Ctrl+C abandons it.
      next = ''
    } else if (char === '\r' || char === '\n') {
      next += '\n'
    } else if (char >= ' ') {
      next += char
    }
  }
  return next.length > TYPED_TASK_BUFFER_MAX_CHARS ? next.slice(-TYPED_TASK_BUFFER_MAX_CHARS) : next
}

// Tail of a terminal reply whose ESC fell in an earlier input chunk, e.g.
// `10;rgb:e6e6/eded/f7f7` (colour query) or `?1;2c` (device attributes).
const TERMINAL_REPLY_REMNANT_PATTERN = /^(?:\d+;rgb:|\??[\d;]+[cRn]\b)/

/** Whether a submitted line looks like a task (not empty, not a `/command`,
 * not a stray terminal reply). */
export function isTypedTaskCandidate(line: string): boolean {
  const trimmed = line.trim()
  return trimmed.length > 0 && !trimmed.startsWith('/') && !TERMINAL_REPLY_REMNANT_PATTERN.test(trimmed)
}
