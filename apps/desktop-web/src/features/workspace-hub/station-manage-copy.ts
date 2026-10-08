import { t, type Locale } from '../../shell/i18n/ui-locale.js'

export interface StationManageModalCopy {
  title: string
  subtitle: string
  submitLabel: string
  deleteLabel: string
  defaultName: string
  namePlaceholder: string
  communicateWithAllLabel: string
  communicateWithAllHint: string
  yoloModeLabel: string
  yoloModeHint: string
}

export function resolveStationManageModalCopy(
  locale: Locale,
  isEdit: boolean,
): StationManageModalCopy {
  return {
    yoloModeLabel: t(locale, 'YOLO MODE（跳过权限确认）', 'YOLO MODE (skip permission prompts)'),
    yoloModeHint: t(
      locale,
      '下次启动时生效。Claude 跳过权限确认；Codex 同时关闭沙盒限制。',
      'Applies on the next launch. Claude skips permission prompts; Codex also disables sandbox restrictions.',
    ),
    title: isEdit ? t(locale, '编辑agent', 'Edit Agent') : t(locale, '新增agent', 'Add Agent'),
    subtitle: isEdit
      ? t(
          locale,
          '更新 agent 的核心属性、角色与执行环境。',
          "Update the agent's core profile, role, and execution environment.",
        )
      : t(
          locale,
          '配置 agent 的核心属性、角色与执行环境。',
          "Configure the agent's core profile, role, and execution environment.",
        ),
    submitLabel: isEdit ? t(locale, '保存', 'Save') : t(locale, '新增agent', 'Add Agent'),
    deleteLabel: t(locale, '删除agent', 'Delete Agent'),
    defaultName: t(locale, '新agent', 'New Agent'),
    namePlaceholder: t(locale, '例如：产品agent-09', 'e.g. Product-Agent-09'),
    communicateWithAllLabel: t(
      locale,
      '允许与专案内所有其他 Agent 沟通',
      'Communicate with all other agents in this project',
    ),
    communicateWithAllHint: t(
      locale,
      '启用后，此 Agent 无需在 Agent Canvas 上连线即可与专案内所有 Agent 互相派发任务，并会以发光外框显示。',
      'When enabled, this agent can exchange tasks with every agent in the project without canvas wires, and is shown with a glowing outline on Agent Canvas.',
    ),
  }
}
