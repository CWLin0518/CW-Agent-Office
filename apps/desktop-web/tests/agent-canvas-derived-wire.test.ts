import test from 'node:test'
import assert from 'node:assert/strict'
import {
  buildAgentCanvasGraph,
  type CanvasNodeInstance,
} from '../src/features/agent-canvas/model/agent-canvas-graph.js'
import {
  loadDerivedEdgesVisible,
  saveDerivedEdgesVisible,
} from '../src/features/agent-canvas/model/agent-canvas-display-preferences.js'

function buildGraph(
  links: unknown[],
  instances: CanvasNodeInstance[] = [
    { instanceId: 'agent-a', agentId: 'agent-a' },
    { instanceId: 'agent-b', agentId: 'agent-b' },
  ],
) {
  return buildAgentCanvasGraph(
    [{ id: 'agent-a' }, { id: 'agent-b' }] as never,
    links as never,
    [],
    instances,
    {},
    {},
    {},
    {},
    {},
    {},
  )
}

function derivedLink(id: string, fromAgentId: string, toAgentId: string) {
  return {
    id,
    workspaceId: 'workspace-1',
    fromAgentId,
    toAgentId,
    kind: 'derived',
    createdAtMs: 1,
  }
}

test('mutual derived links collapse into one bidirectional canvas edge', () => {
  const graph = buildGraph([
    derivedLink('newer', 'agent-b', 'agent-a'),
    derivedLink('older', 'agent-a', 'agent-b'),
  ])
  const links = graph.edges.filter((edge) => edge.data.kind === 'link')

  assert.equal(links.length, 1)
  assert.equal(links[0]?.data.kind === 'link' && links[0].data.bidirectional, true)
  assert.equal(links[0]?.fromId, 'agent-a')
  assert.equal(links[0]?.toId, 'agent-b')
})

test('one-way derived traffic remains a single-headed canvas edge', () => {
  const graph = buildGraph([derivedLink('one-way', 'agent-b', 'agent-a')])
  const link = graph.edges.find((edge) => edge.data.kind === 'link')

  assert.equal(link?.data.kind === 'link' && link.data.bidirectional, false)
})

test('mutual derived wire follows canvas left-to-right position instead of agent id', () => {
  const graph = buildGraph(
    [
      derivedLink('a-to-b', 'agent-a', 'agent-b'),
      derivedLink('b-to-a', 'agent-b', 'agent-a'),
    ],
    [
      { instanceId: 'agent-a', agentId: 'agent-a', position: { x: 640, y: 80 } },
      { instanceId: 'agent-b', agentId: 'agent-b', position: { x: 80, y: 80 } },
    ],
  )
  const link = graph.edges.find((edge) => edge.data.kind === 'link')

  assert.equal(link?.fromId, 'agent-b')
  assert.equal(link?.toId, 'agent-a')
  assert.equal(link?.data.kind === 'link' && link.data.bidirectional, true)
})

test('derived-wire visibility survives a canvas pane remount and stays workspace scoped', () => {
  const values = new Map<string, string>()
  const originalWindow = globalThis.window
  Object.defineProperty(globalThis, 'window', {
    configurable: true,
    value: {
      localStorage: {
        getItem: (key: string) => values.get(key) ?? null,
        setItem: (key: string, value: string) => values.set(key, value),
      },
    },
  })
  try {
    saveDerivedEdgesVisible('workspace-1', false)
    assert.equal(loadDerivedEdgesVisible('workspace-1'), false)
    assert.equal(loadDerivedEdgesVisible('workspace-2'), true)
  } finally {
    Object.defineProperty(globalThis, 'window', { configurable: true, value: originalWindow })
  }
})
