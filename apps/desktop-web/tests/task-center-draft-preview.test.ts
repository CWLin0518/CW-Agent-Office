import test from 'node:test'
import assert from 'node:assert/strict'
import {
  buildTaskCenterWorkspaceSnapshot,
  groupTaskSendPreviewTargets,
  insertTaskDraftPreviewAtCursor,
  parseTaskCenterWorkspaceSnapshot,
  serializeTaskCenterWorkspaceSnapshot,
  splitRestoredTaskDraftIntoPreview,
} from '../src/features/task-center/task-center-model.js'

test('splitRestoredTaskDraftIntoPreview moves a restored unsent draft into the preview, never the live editor', () => {
  const restored = splitRestoredTaskDraftIntoPreview({
    markdown: '# 上次没发出去的任务',
    previewMarkdown: '',
    targetStationIds: ['a'],
  })
  assert.equal(restored.markdown, '')
  assert.equal(restored.previewMarkdown, '# 上次没发出去的任务')
  assert.deepEqual(restored.targetStationIds, ['a'])
})

test('splitRestoredTaskDraftIntoPreview falls back to a previously-restored preview when markdown is blank', () => {
  const restored = splitRestoredTaskDraftIntoPreview({
    markdown: '   ',
    previewMarkdown: '之前留下的草稿',
    targetStationIds: [],
  })
  assert.equal(restored.markdown, '')
  assert.equal(restored.previewMarkdown, '之前留下的草稿')
})

test('splitRestoredTaskDraftIntoPreview merges an unconsumed preview with a newer unsent draft instead of dropping either', () => {
  // A user can leave a session with both an in-progress edit (markdown) and an
  // older preview they never accepted or dismissed (previewMarkdown). Neither
  // should be silently lost on the next restore.
  const restored = splitRestoredTaskDraftIntoPreview({
    markdown: 'bar',
    previewMarkdown: 'foo',
    targetStationIds: [],
  })
  assert.equal(restored.markdown, '')
  assert.equal(restored.previewMarkdown, 'foo\n\n---\n\nbar')
})

test('splitRestoredTaskDraftIntoPreview leaves a fresh empty draft with no preview', () => {
  const restored = splitRestoredTaskDraftIntoPreview({
    markdown: '',
    previewMarkdown: '',
    targetStationIds: [],
  })
  assert.equal(restored.markdown, '')
  assert.equal(restored.previewMarkdown, '')
})

test('insertTaskDraftPreviewAtCursor pastes the preview at the cursor without touching the rest of the input', () => {
  const prefix = '已经打好的开头 '
  const result = insertTaskDraftPreviewAtCursor(prefix, '带入的默认文字', prefix.length)
  assert.equal(result.markdown, '已经打好的开头 带入的默认文字')
  assert.equal(result.cursor, prefix.length + '带入的默认文字'.length)
})

test('insertTaskDraftPreviewAtCursor inserts mid-string at an interior cursor', () => {
  const result = insertTaskDraftPreviewAtCursor('AB', 'X', 1)
  assert.equal(result.markdown, 'AXB')
  assert.equal(result.cursor, 2)
})

test('insertTaskDraftPreviewAtCursor clamps an out-of-range cursor', () => {
  const result = insertTaskDraftPreviewAtCursor('abc', 'X', 999)
  assert.equal(result.markdown, 'abcX')
  assert.equal(result.cursor, 4)
})

test('preview markdown round-trips through the persisted workspace snapshot', () => {
  const snapshot = buildTaskCenterWorkspaceSnapshot({
    updatedAtMs: 1700000000000,
    draft: {
      markdown: '',
      previewMarkdown: '未发送的草稿内容',
      targetStationIds: ['b'],
    },
    dispatchHistory: [],
  })
  const parsed = parseTaskCenterWorkspaceSnapshot(serializeTaskCenterWorkspaceSnapshot(snapshot))
  assert.equal(parsed?.draft.previewMarkdown, '未发送的草稿内容')
  assert.equal(parsed?.draft.markdown, '')
})

test('groupTaskSendPreviewTargets drops targets with nothing appended', () => {
  const groups = groupTaskSendPreviewTargets([
    { targetAgentId: 'agent-1', appendedText: '' },
    { targetAgentId: 'agent-2', appendedText: '   ' },
  ])
  assert.deepEqual(groups, [])
})

test('groupTaskSendPreviewTargets merges targets that get the exact same appended text', () => {
  const groups = groupTaskSendPreviewTargets([
    { targetAgentId: 'agent-1', appendedText: '## GT Office Output\n\n...' },
    { targetAgentId: 'agent-2', appendedText: '## GT Office Output\n\n...' },
  ])
  assert.deepEqual(groups, [
    { appendedText: '## GT Office Output\n\n...', targetAgentIds: ['agent-1', 'agent-2'] },
  ])
})

test('groupTaskSendPreviewTargets keeps distinct appended texts as separate groups', () => {
  const groups = groupTaskSendPreviewTargets([
    { targetAgentId: 'agent-1', appendedText: 'A' },
    { targetAgentId: 'agent-2', appendedText: '' },
    { targetAgentId: 'agent-3', appendedText: 'B' },
  ])
  assert.deepEqual(groups, [
    { appendedText: 'A', targetAgentIds: ['agent-1'] },
    { appendedText: 'B', targetAgentIds: ['agent-3'] },
  ])
})

test('parsing an older snapshot without previewMarkdown defaults it to empty', () => {
  const legacyRaw = JSON.stringify({
    version: 2,
    updatedAtMs: 1700000000000,
    draft: {
      markdown: '旧版草稿',
      targetStationIds: ['a'],
    },
    dispatchHistory: [],
  })
  const parsed = parseTaskCenterWorkspaceSnapshot(legacyRaw)
  assert.equal(parsed?.draft.markdown, '旧版草稿')
  assert.equal(parsed?.draft.previewMarkdown, '')
})
