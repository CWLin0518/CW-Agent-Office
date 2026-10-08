import {
  cancelStationTerminalFrameFlush,
  scheduleStationTerminalFrameFlush,
  type StationTerminalFrameFlushHandle,
  type StationTerminalFrameFlushScheduler,
} from './station-terminal-frame-flush-scheduler.js'

export interface StationTerminalResizeDimensions {
  cols: number
  rows: number
}

export interface StationTerminalFitRetryFrame {
  handle: StationTerminalFrameFlushHandle | null
  cancel: () => void
}

export interface ScheduleStationTerminalFitRetryFrameOptions {
  scheduler: StationTerminalFrameFlushScheduler
  run: () => void
  fallbackDelayMs?: number
}

const TERMINAL_RESIZE_MAX_DIMENSION = 65_535

export function normalizeStationTerminalResizeDimensions(
  cols: number,
  rows: number,
): StationTerminalResizeDimensions | null {
  if (!Number.isFinite(cols) || !Number.isFinite(rows)) {
    return null
  }

  const normalizedCols = Math.floor(cols)
  const normalizedRows = Math.floor(rows)
  if (
    normalizedCols < 1 ||
    normalizedRows < 1 ||
    normalizedCols > TERMINAL_RESIZE_MAX_DIMENSION ||
    normalizedRows > TERMINAL_RESIZE_MAX_DIMENSION
  ) {
    return null
  }

  return {
    cols: normalizedCols,
    rows: normalizedRows,
  }
}

export function scheduleStationTerminalFitRetryFrame({
  scheduler,
  run,
  fallbackDelayMs,
}: ScheduleStationTerminalFitRetryFrameOptions): StationTerminalFitRetryFrame {
  const retryFrame: StationTerminalFitRetryFrame = {
    handle: null,
    cancel: () => {},
  }
  retryFrame.cancel = () => {
    cancelStationTerminalFrameFlush(retryFrame.handle)
    retryFrame.handle = null
  }
  retryFrame.handle = scheduleStationTerminalFrameFlush(
    () => {
      retryFrame.handle = null
      run()
    },
    scheduler,
    fallbackDelayMs,
  )
  return retryFrame
}

export interface StationTerminalResizeTimerApi {
  setTimeout: (callback: () => void, delayMs: number) => number
  clearTimeout: (id: number) => void
}

export type StationTerminalResizeSender = (cols: number, rows: number) => Promise<unknown>

export interface StationTerminalResizeCoalescer {
  request: (key: string, cols: number, rows: number, send: StationTerminalResizeSender) => void
  dispose: () => void
}

interface StationTerminalResizeCoalescerEntry {
  pending: { cols: number; rows: number; send: StationTerminalResizeSender } | null
  timerId: number | null
  inFlight: boolean
}

export const STATION_TERMINAL_RESIZE_SETTLE_MS = 60

// A PTY resize triggers SIGWINCH and a full redraw in TUI agents. Fire-and-forget
// resize calls during a drag can reach the backend out of order, leaving the PTY
// at a stale width while xterm already reflowed to the new one, which shows up as
// overlapping or truncated lines. Debounce per session and keep at most one
// request in flight so the last size the user settled on always wins.
export function createStationTerminalResizeCoalescer(
  timers: StationTerminalResizeTimerApi,
  settleMs: number = STATION_TERMINAL_RESIZE_SETTLE_MS,
): StationTerminalResizeCoalescer {
  const entries = new Map<string, StationTerminalResizeCoalescerEntry>()

  const flush = (key: string) => {
    const entry = entries.get(key)
    if (!entry) {
      return
    }
    entry.timerId = null
    if (entry.inFlight) {
      return
    }
    const next = entry.pending
    entry.pending = null
    if (!next) {
      entries.delete(key)
      return
    }
    entry.inFlight = true
    void Promise.resolve()
      .then(() => next.send(next.cols, next.rows))
      .catch(() => {
        // Resize failures are non-critical; the next resize re-syncs the PTY.
      })
      .finally(() => {
        entry.inFlight = false
        if (entries.get(key) !== entry) {
          return
        }
        if (entry.pending && entry.timerId === null) {
          flush(key)
          return
        }
        if (!entry.pending && entry.timerId === null) {
          entries.delete(key)
        }
      })
  }

  return {
    request: (key, cols, rows, send) => {
      const entry = entries.get(key) ?? { pending: null, timerId: null, inFlight: false }
      entries.set(key, entry)
      entry.pending = { cols, rows, send }
      if (entry.timerId !== null) {
        timers.clearTimeout(entry.timerId)
      }
      entry.timerId = timers.setTimeout(() => flush(key), settleMs)
    },
    // Drops queued work without poisoning the coalescer, so a StrictMode
    // effect re-run can keep using the same instance.
    dispose: () => {
      for (const entry of entries.values()) {
        if (entry.timerId !== null) {
          timers.clearTimeout(entry.timerId)
        }
      }
      entries.clear()
    },
  }
}
