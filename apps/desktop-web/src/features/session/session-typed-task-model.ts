// Rebuilds the line a user is typing into a station terminal from raw xterm
// input, so the session can be named after the first prompt they submit.
// Approximate by design: it only needs to recover readable task text.

const TYPED_TASK_BUFFER_MAX_CHARS = 2000
// CSI (incl. bracketed-paste markers and arrow keys), SS3 and two-char escapes.
// eslint-disable-next-line no-control-regex
const ESCAPE_SEQUENCE_PATTERN = /\x1b\[[0-9;?]*[ -/]*[@-~]|\x1bO.|\x1b./g

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

/** Whether a submitted line looks like a task (not empty, not a `/command`). */
export function isTypedTaskCandidate(line: string): boolean {
  const trimmed = line.trim()
  return trimmed.length > 0 && !trimmed.startsWith('/')
}
