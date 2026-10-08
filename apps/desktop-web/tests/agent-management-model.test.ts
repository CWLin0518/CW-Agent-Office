import test from 'node:test'
import assert from 'node:assert/strict'

import {
  applyModelToLaunchCommand,
  applyProviderToLaunchCommand,
  applyYoloToLaunchCommand,
  isYoloLaunchCommand,
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

test('switching providers resets the executable, model and provider-specific flags in both directions', () => {
  const claudeCommand = applyProviderToLaunchCommand('codex --model gpt-5.1 -p work', 'codex', 'claude')
  assert.equal(claudeCommand, '')
  assert.equal(applyModelToLaunchCommand(claudeCommand, 'claude', 'opus'), 'claude --model opus')

  const codexCommand = applyProviderToLaunchCommand('claude --model opus --permission-mode plan', 'claude', 'codex')
  assert.equal(codexCommand, '')
  assert.equal(applyModelToLaunchCommand(codexCommand, 'codex', 'gpt-5.1'), 'codex --model gpt-5.1')
  assert.equal(applyProviderToLaunchCommand('', 'codex', 'claude'), '')
  assert.equal(applyProviderToLaunchCommand('custom-wrapper --model opus', 'claude', 'codex'), '')
  assert.equal(applyProviderToLaunchCommand('codex -p work', 'codex', 'codex'), 'codex -p work')
})

test('YOLO toggles round-trip without losing model or custom command arguments', () => {
  for (const provider of ['codex', 'claude'] as const) {
    const command = `${provider} --model test-model --resume "session name"`
    const enabled = applyYoloToLaunchCommand(command, provider, true)
    assert.equal(isYoloLaunchCommand(enabled, provider), true)
    assert.equal(applyYoloToLaunchCommand(enabled, provider, true), enabled)
    assert.equal(applyYoloToLaunchCommand(enabled, provider, false), command)
    assert.equal(applyYoloToLaunchCommand(applyYoloToLaunchCommand('', provider, true), provider, false), '')
  }
})

test('YOLO aliases, quoted prompt text and provider switching stay consistent', () => {
  assert.equal(isYoloLaunchCommand('codex --yolo', 'codex'), true)
  assert.equal(applyYoloToLaunchCommand('codex --yolo', 'codex', false), '')
  assert.equal(isYoloLaunchCommand('claude --permission-mode bypassPermissions', 'claude'), true)
  assert.equal(applyYoloToLaunchCommand('claude --permission-mode=bypassPermissions --model opus', 'claude', false), 'claude --model opus')
  assert.equal(applyYoloToLaunchCommand('claude --permission-mode bypassPermissions', 'claude', false), '')
  assert.equal(isYoloLaunchCommand('claude "explain --dangerously-skip-permissions"', 'claude'), false)
  assert.equal(applyYoloToLaunchCommand('claude "explain --dangerously-skip-permissions"', 'claude', false), 'claude "explain --dangerously-skip-permissions"')
  const claude = applyProviderToLaunchCommand('codex --yolo --model old', 'codex', 'claude')
  assert.equal(claude, 'claude --dangerously-skip-permissions')
  assert.equal(applyProviderToLaunchCommand(claude, 'claude', 'codex'), 'codex --dangerously-bypass-approvals-and-sandbox')
  assert.equal(applyModelToLaunchCommand(claude, 'claude', 'opus'), 'claude --dangerously-skip-permissions --model opus')
})

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
