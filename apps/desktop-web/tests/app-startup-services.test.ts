import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

const mainSource = readFileSync(resolve(process.cwd(), 'src/main.tsx'), 'utf8')

test('desktop startup signals the backend before rendering the React shell', () => {
  const tauriGuardIndex = mainSource.indexOf('if (desktopApi.isTauriRuntime())')
  const signalIndex = mainSource.indexOf('desktopApi.signalUiReady()')
  const renderIndex = mainSource.indexOf('createRoot(rootElement).render(')

  assert.notEqual(tauriGuardIndex, -1)
  assert.notEqual(signalIndex, -1)
  assert.notEqual(renderIndex, -1)
  assert.ok(tauriGuardIndex < signalIndex)
  assert.ok(signalIndex < renderIndex)
})
