import test from 'node:test'
import assert from 'node:assert/strict'
import {
  CONPTY_REFLOW_MIN_BUILD,
  buildStationTerminalWindowsPty,
  detectStationTerminalWindowsPty,
  resolveWindowsBuildNumberFromPlatformVersion,
} from '../src/features/terminal/station-terminal-windows-pty.js'

test('maps UA-CH platformVersion to a Windows build on the right side of the ConPTY reflow cutoff', () => {
  const windows11 = resolveWindowsBuildNumberFromPlatformVersion('15.0.0')
  const windows10 = resolveWindowsBuildNumberFromPlatformVersion('10.0.0')
  assert.ok(windows11 !== undefined && windows11 >= CONPTY_REFLOW_MIN_BUILD)
  assert.ok(windows10 !== undefined && windows10 < CONPTY_REFLOW_MIN_BUILD)
  assert.equal(resolveWindowsBuildNumberFromPlatformVersion('0.3.0'), undefined)
  assert.equal(resolveWindowsBuildNumberFromPlatformVersion(''), undefined)
})

test('builds conservative ConPTY options when the build is unknown', () => {
  assert.deepEqual(buildStationTerminalWindowsPty(), { backend: 'conpty' })
  assert.deepEqual(buildStationTerminalWindowsPty(22000), { backend: 'conpty', buildNumber: 22000 })
})

test('detects the ConPTY build from userAgentData once and caches it', async () => {
  let calls = 0
  const userAgentData = {
    getHighEntropyValues: async () => {
      calls += 1
      return { platformVersion: '15.0.0' }
    },
  }
  const first = await detectStationTerminalWindowsPty(userAgentData)
  const second = await detectStationTerminalWindowsPty(undefined)
  assert.deepEqual(first, { backend: 'conpty', buildNumber: 22000 })
  assert.equal(second, first)
  assert.equal(calls, 1)
})
