import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'

const testDir = dirname(fileURLToPath(import.meta.url))
const scss = readFileSync(resolve(testDir, '../../src/features/agent-canvas/AgentCanvasPane.scss'), 'utf8')

test('agent canvas output preview has opaque backgrounds for both dark themes', () => {
  const dialogStart = scss.indexOf('.agent-canvas-output-preview-dialog')
  const bodyStart = scss.indexOf('.agent-canvas-output-preview-body', dialogStart)
  const dialogStyles = scss.slice(dialogStart, bodyStart)

  assert.match(
    dialogStyles,
    /:root\[data-theme='graphite-dark'\] & \{\s*background: #1c1c1e;/,
  )
  assert.match(
    dialogStyles,
    /:root\[data-theme='sakura-night'\] & \{\s*background: #1c0e1a;/,
  )
  assert.match(dialogStyles, /color: var\(--vb-text\);/)
})
