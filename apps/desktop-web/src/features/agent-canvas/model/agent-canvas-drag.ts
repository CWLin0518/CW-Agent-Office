/** Custom drag MIME type carrying an `AgentProfile.id` from the standby
 * rail (`AgentCanvasStandbyRail`) to the canvas (`AgentCanvasPane`) —
 * native HTML5 drag-and-drop, not the pointer-based drag this pane's own
 * wire/node dragging uses, since the source and drop target here are
 * separate shell panes (left rail vs. main canvas) with no shared parent
 * state to route pointer capture through; the browser already handles
 * cross-pane hit-testing for a native drag session (docs/cw/04_客製化設計.md
 * §8, P4.6). */
export const AGENT_CANVAS_DRAG_MIME_TYPE = 'application/x-agent-canvas-agent-id'
