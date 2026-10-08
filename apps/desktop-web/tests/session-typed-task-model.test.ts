import test from 'node:test'
import assert from 'node:assert/strict'

import {
  appendTypedTaskInput,
  isTypedTaskCandidate,
} from '../src/features/session/session-typed-task-model.js'

test('rebuilds typed text, honouring backspace and stripping escape sequences', () => {
  let buffer = ''
  for (const data of ['F', 'i', 'x', 'x', '\x7f', ' ', '\x1b[D', 'login', '\x1b[200~ bug\x1b[201~']) {
    buffer = appendTypedTaskInput(buffer, data)
  }
  assert.equal(buffer, 'Fix login bug')
})

test('ctrl+u and ctrl+c clear the pending line', () => {
  assert.equal(appendTypedTaskInput('draft', '\x15'), '')
  assert.equal(appendTypedTaskInput('draft', '\x03next'), 'next')
})

test('only real prompts count as a first task', () => {
  assert.equal(isTypedTaskCandidate('  修复登录  '), true)
  assert.equal(isTypedTaskCandidate('/model'), false)
  assert.equal(isTypedTaskCandidate('   '), false)
})
