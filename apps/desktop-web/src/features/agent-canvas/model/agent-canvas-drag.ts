/**
 * Native HTML5 drag-and-drop payload carrying an `AgentProfile.id` from the
 * standby rail (`AgentCanvasStandbyRail`) to the canvas (`AgentCanvasPane`)
 * — not the pointer-based drag this pane's own wire/node dragging uses,
 * since the source and drop target here are separate shell panes (left
 * rail vs. main canvas) with no shared parent state to route pointer
 * capture through; the browser already handles cross-pane hit-testing for
 * a native drag session (docs/cw/04_客製化設計.md §8, P4.6).
 *
 * Deliberately uses the standard `text/plain` MIME type with a
 * distinguishing prefix, NOT a custom MIME type (`application/x-...`) —
 * custom types passed to `dataTransfer.setData`/read via `.types` are
 * unreliable across WebView2/Chromium versions in practice (a known,
 * broadly-reported native-drag-and-drop gotcha), whereas `text/plain` is
 * universally supported. The prefix keeps this from misfiring if some
 * unrelated plain-text drag (e.g. dragging selected text) ever lands on
 * the canvas.
 */
const AGENT_CANVAS_DRAG_PREFIX = 'agent-canvas-agent-id:'

export function setAgentCanvasDragPayload(dataTransfer: DataTransfer, agentId: string): void {
  dataTransfer.setData('text/plain', `${AGENT_CANVAS_DRAG_PREFIX}${agentId}`)
}

/** Only readable at `drop` (not `dragover`/`dragenter` — per the HTML5 DnD
 * spec, `getData` returns `''` during those for security, so callers must
 * gate `dragover`'s `preventDefault()` some other way; this pane simply
 * always allows the drop and lets this parser no-op on a non-match). */
export function parseAgentCanvasDragPayload(dataTransfer: DataTransfer): string | null {
  const raw = dataTransfer.getData('text/plain')
  return raw.startsWith(AGENT_CANVAS_DRAG_PREFIX) ? raw.slice(AGENT_CANVAS_DRAG_PREFIX.length) : null
}
