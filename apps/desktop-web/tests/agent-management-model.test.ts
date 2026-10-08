import test from 'node:test'
import assert from 'node:assert/strict'

import {
  buildDefaultAgentWorkdir,
  isWorkspaceRootAgentWorkdir,
  resolveAgentModelDisplayLabel,
  resolveAvailableAgentProviders,
  resolveInitialAgentModel,
  resolveManagedProviderKey,
  resolveModelOptionsForProvider,
  resolvePromptFileRelativePathForProvider,
  resolvePromptFileNameForProvider,
} from '../src/features/workspace-hub/agent-management-model.js'

test('builds the new shallow default agent workdir', () => {
  assert.equal(buildDefaultAgentWorkdir('My Product Agent'), '.')
  assert.equal(buildDefaultAgentWorkdir('  Claude负责人  '), '.')
})

test('maps providers to the correct system prompt filenames', () => {
  assert.equal(resolvePromptFileNameForProvider('claude'), 'CLAUDE.md')
  assert.equal(resolvePromptFileNameForProvider('codex'), 'AGENTS.md')
})

test('resolves prompt file paths against the selected workdir', () => {
  assert.equal(resolvePromptFileRelativePathForProvider('codex', '.'), 'AGENTS.md')
  assert.equal(resolvePromptFileRelativePathForProvider('claude', '.gtoffice/research'), '.gtoffice/research/CLAUDE.md')
})

test('defaults unknown tool strings to codex', () => {
  assert.equal(resolveManagedProviderKey('other-tool'), 'codex')
  assert.equal(resolveManagedProviderKey(''), 'codex')
})

test('recognizes workspace-root agent workdirs', () => {
  assert.equal(isWorkspaceRootAgentWorkdir('.'), true)
  assert.equal(isWorkspaceRootAgentWorkdir(''), true)
  assert.equal(isWorkspaceRootAgentWorkdir('.gtoffice/research'), false)
})

test('only exposes configured or installed providers for the agent form', () => {
  const providers = resolveAvailableAgentProviders([
    {
      agent: 'claude',
      installStatus: {
        installed: true,
      },
      configStatus: 'guidance_only',
    },
    {
      agent: 'codex',
      installStatus: {
        installed: false,
      },
      configStatus: 'configured',
    },
    {
      agent: 'openai',
      installStatus: {
        installed: true,
      },
      configStatus: 'configured',
    },
  ])

  assert.deepEqual(
    providers.map((item) => item.key),
    ['claude', 'codex'],
  )
})

test('prefers CLI-discovered models and falls back to version-tracking aliases', () => {
  const discovered = [{ value: 'gpt-9-codex', label: 'GPT-9 Codex (gpt-9-codex)', shortLabel: 'GPT-9 Codex' }]
  assert.deepEqual(resolveModelOptionsForProvider('codex', discovered), discovered)
  assert.equal(resolveModelOptionsForProvider('claude', null)[0].value, 'opus')
  assert.equal(resolveModelOptionsForProvider('claude', [])[0].value, 'opus')
})

test('keeps an existing agent model selectable even when the CLI no longer lists it', () => {
  assert.equal(resolveInitialAgentModel('claude', 'claude --model claude-opus-5'), 'claude-opus-5')
  const options = resolveModelOptionsForProvider('claude', null, 'claude-opus-5')
  assert.equal(options[options.length - 1].shortLabel, 'Opus 5')
  assert.equal(resolveAgentModelDisplayLabel('claude', 'claude --model sonnet'), 'Sonnet')
})
