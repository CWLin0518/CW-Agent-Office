/**
 * Shared node/wire color palette for agent-canvas (docs/cw/04_客製化設計.md §1,
 * P4.6). `null` is deliberately not a member of this array — it means
 * "default gray," i.e. no color set, which callers offer as a separate
 * "reset" action rather than a 6th preset value.
 */
export const AGENT_CANVAS_COLOR_PRESETS = [
  '#ef4444', // red
  '#f59e0b', // amber
  '#22c55e', // green
  '#3b82f6', // blue
  '#a855f7', // purple
] as const
