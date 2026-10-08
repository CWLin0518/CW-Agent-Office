import test from 'node:test'
import assert from 'node:assert/strict'
import {
  isWindowsWebViewEnvironment,
  shouldUseStationTerminalWebglRenderer,
} from '../src/features/terminal/station-terminal-renderer-policy.js'

test('keeps macOS WebKit terminals on the stable default renderer', () => {
  assert.equal(shouldUseStationTerminalWebglRenderer({ isMacOsWebKit: true, isWindowsWebView: false }), false)
})

test('keeps Windows WebView2 terminals on the stable default renderer', () => {
  assert.equal(shouldUseStationTerminalWebglRenderer({ isMacOsWebKit: false, isWindowsWebView: true }), false)
})

test('keeps WebGL enabled elsewhere', () => {
  assert.equal(shouldUseStationTerminalWebglRenderer({ isMacOsWebKit: false, isWindowsWebView: false }), true)
})

test('detects Windows WebView2 from the user agent', () => {
  assert.equal(
    isWindowsWebViewEnvironment(
      'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36 Edg/130.0.0.0',
    ),
    true,
  )
  assert.equal(
    isWindowsWebViewEnvironment('Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)'),
    false,
  )
})
