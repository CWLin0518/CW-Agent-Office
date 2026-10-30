import test from 'node:test'
import assert from 'node:assert/strict'
import { submitSessionResume } from '../src/features/terminal/submit-session-resume.js'

test('resume submits exactly once with plain Enter after the paste settles', async () => {
  const calls: Array<string | number> = []
  assert.equal(await submitSessionResume({
    prompt: 'continue',
    isCurrentSession: () => true,
    write: async (input) => { calls.push(input); return true },
    sleep: async (ms) => { calls.push(ms) },
  }), true)
  assert.deepEqual(calls, ['continue', 500, '\r'])
})

test('switching sessions during paste never submits to the replacement session', async () => {
  let current = true
  const writes: string[] = []
  assert.equal(await submitSessionResume({
    prompt: 'continue',
    isCurrentSession: () => current,
    write: async (input) => { writes.push(input); return true },
    sleep: async () => { current = false },
  }), false)
  assert.deepEqual(writes, ['continue'])
})

test('a rejected prompt is never followed by Enter', async () => {
  const writes: string[] = []
  assert.equal(await submitSessionResume({
    prompt: 'continue',
    isCurrentSession: () => true,
    write: async (input) => { writes.push(input); return false },
    sleep: async () => { assert.fail('must not wait after rejection') },
  }), false)
  assert.deepEqual(writes, ['continue'])
})
