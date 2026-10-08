import test from 'node:test'
import assert from 'node:assert/strict'
import {
  createStationTerminalResizeCoalescer,
  normalizeStationTerminalResizeDimensions,
  scheduleStationTerminalFitRetryFrame,
} from '../src/features/terminal/station-terminal-resize.js'
import type { StationTerminalFrameFlushScheduler } from '../src/features/terminal/station-terminal-frame-flush-scheduler.js'

function createFakeScheduler() {
  let nextId = 1
  const frameCallbacks = new Map<number, FrameRequestCallback>()
  const timeoutCallbacks = new Map<number, { callback: () => void; delayMs: number }>()
  const cancelledFrames: number[] = []
  const clearedTimeouts: number[] = []
  const scheduler: StationTerminalFrameFlushScheduler = {
    requestAnimationFrame: (callback) => {
      const id = nextId
      nextId += 1
      frameCallbacks.set(id, callback)
      return id
    },
    cancelAnimationFrame: (id) => {
      cancelledFrames.push(id)
      frameCallbacks.delete(id)
    },
    setTimeout: (callback, delayMs) => {
      const id = nextId
      nextId += 1
      timeoutCallbacks.set(id, { callback, delayMs })
      return id
    },
    clearTimeout: (id) => {
      clearedTimeouts.push(id)
      timeoutCallbacks.delete(id)
    },
  }

  return {
    scheduler,
    frameCallbacks,
    timeoutCallbacks,
    cancelledFrames,
    clearedTimeouts,
  }
}

test('normalizes terminal resize dimensions after flooring', () => {
  assert.deepEqual(normalizeStationTerminalResizeDimensions(120.9, 40.1), {
    cols: 120,
    rows: 40,
  })
})

test('drops terminal resize dimensions that floor below one', () => {
  assert.equal(normalizeStationTerminalResizeDimensions(0.9, 24), null)
  assert.equal(normalizeStationTerminalResizeDimensions(80, 0.9), null)
})

test('drops non-finite or u16-out-of-range terminal resize dimensions', () => {
  assert.equal(normalizeStationTerminalResizeDimensions(Number.NaN, 24), null)
  assert.equal(normalizeStationTerminalResizeDimensions(80, Number.POSITIVE_INFINITY), null)
  assert.equal(normalizeStationTerminalResizeDimensions(65_536, 24), null)
  assert.equal(normalizeStationTerminalResizeDimensions(80, 65_536), null)
})

test('terminal fit retry frame runs on the next animation frame and clears fallback', () => {
  const fake = createFakeScheduler()
  let runCount = 0
  const retry = scheduleStationTerminalFitRetryFrame({
    scheduler: fake.scheduler,
    fallbackDelayMs: 18,
    run: () => {
      runCount += 1
    },
  })

  assert.equal(retry.handle?.frameId, 1)
  assert.equal(retry.handle?.timeoutId, 2)
  assert.equal(fake.timeoutCallbacks.get(2)?.delayMs, 18)
  fake.frameCallbacks.get(1)?.(16)
  fake.timeoutCallbacks.get(2)?.callback()

  assert.equal(runCount, 1)
  assert.equal(retry.handle, null)
  assert.deepEqual(fake.clearedTimeouts, [2])
})

test('terminal fit retry frame falls back when animation frames are throttled', () => {
  const fake = createFakeScheduler()
  let runCount = 0
  const retry = scheduleStationTerminalFitRetryFrame({
    scheduler: fake.scheduler,
    fallbackDelayMs: 18,
    run: () => {
      runCount += 1
    },
  })

  fake.timeoutCallbacks.get(2)?.callback()
  fake.frameCallbacks.get(1)?.(16)

  assert.equal(runCount, 1)
  assert.equal(retry.handle, null)
  assert.deepEqual(fake.cancelledFrames, [1])
})

test('terminal fit retry frame cancellation clears pending callbacks', () => {
  const fake = createFakeScheduler()
  let runCount = 0
  const retry = scheduleStationTerminalFitRetryFrame({
    scheduler: fake.scheduler,
    fallbackDelayMs: 18,
    run: () => {
      runCount += 1
    },
  })

  retry.cancel()
  fake.frameCallbacks.get(1)?.(16)
  fake.timeoutCallbacks.get(2)?.callback()

  assert.equal(runCount, 0)
  assert.equal(retry.handle, null)
  assert.deepEqual(fake.cancelledFrames, [1])
  assert.deepEqual(fake.clearedTimeouts, [2])
})

function createFakeResizeTimers() {
  let nextId = 1
  const callbacks = new Map<number, () => void>()
  return {
    timers: {
      setTimeout: (callback: () => void) => {
        const id = nextId
        nextId += 1
        callbacks.set(id, callback)
        return id
      },
      clearTimeout: (id: number) => {
        callbacks.delete(id)
      },
    },
    runAll: () => {
      const pending = [...callbacks.entries()]
      callbacks.clear()
      for (const [, callback] of pending) {
        callback()
      }
    },
    pendingCount: () => callbacks.size,
  }
}

const flushMicrotasks = () => new Promise<void>((resolve) => setImmediate(resolve))

test('resize coalescer only sends the last size requested within the settle window', async () => {
  const { timers, runAll } = createFakeResizeTimers()
  const coalescer = createStationTerminalResizeCoalescer(timers)
  const sent: Array<[number, number]> = []
  const send = async (cols: number, rows: number) => {
    sent.push([cols, rows])
  }
  coalescer.request('ws::s1', 80, 24, send)
  coalescer.request('ws::s1', 100, 30, send)
  coalescer.request('ws::s1', 120, 40, send)
  runAll()
  await flushMicrotasks()
  assert.deepEqual(sent, [[120, 40]])
})

test('resize coalescer keeps one request in flight and sends the latest size after it settles', async () => {
  const { timers, runAll } = createFakeResizeTimers()
  const coalescer = createStationTerminalResizeCoalescer(timers)
  const sent: Array<[number, number]> = []
  let releaseFirst: (() => void) | null = null
  const send = (cols: number, rows: number) => {
    sent.push([cols, rows])
    if (sent.length === 1) {
      return new Promise<void>((resolve) => {
        releaseFirst = resolve
      })
    }
    return Promise.resolve()
  }
  coalescer.request('ws::s1', 80, 24, send)
  runAll()
  await flushMicrotasks()
  coalescer.request('ws::s1', 90, 24, send)
  coalescer.request('ws::s1', 132, 43, send)
  runAll()
  await flushMicrotasks()
  assert.deepEqual(sent, [[80, 24]])
  releaseFirst!()
  await flushMicrotasks()
  assert.deepEqual(sent, [[80, 24], [132, 43]])
})

test('resize coalescer survives failed sends and dispose', async () => {
  const { timers, runAll, pendingCount } = createFakeResizeTimers()
  const coalescer = createStationTerminalResizeCoalescer(timers)
  const sent: Array<[number, number]> = []
  coalescer.request('ws::s1', 80, 24, async () => {
    throw new Error('boom')
  })
  runAll()
  await flushMicrotasks()
  coalescer.request('ws::s2', 70, 20, async (cols, rows) => {
    sent.push([cols, rows])
  })
  coalescer.dispose()
  assert.equal(pendingCount(), 0)
  coalescer.request('ws::s1', 100, 30, async (cols, rows) => {
    sent.push([cols, rows])
  })
  runAll()
  await flushMicrotasks()
  assert.deepEqual(sent, [[100, 30]])
})
