import { t, type Locale } from '@shell/i18n/ui-locale'
import { AppIcon } from '@shell/ui/icons'
import { AGENT_CANVAS_COLOR_PRESETS } from '../model/agent-canvas-colors'

interface AgentCanvasColorSwatchesProps {
  locale: Locale
  /** The color shared by every currently-selected node/wire, or `null` when
   * they disagree (or all use the default) — drives which swatch (if any)
   * shows the "active" checkmark. */
  activeColor: string | null
  onPick: (color: string | null) => void
}

/** Row of preset color swatches plus a "reset to default" swatch — shared by
 * both the node context menu (agent border color) and the wire context menu
 * (authored-link color), so both present the exact same palette
 * (docs/cw/04_客製化設計.md §8, P4.6). */
export function AgentCanvasColorSwatches({ locale, activeColor, onPick }: AgentCanvasColorSwatchesProps) {
  return (
    <div className="agent-canvas-color-swatches" role="group" aria-label={t(locale, 'agentCanvas.color.groupLabel')}>
      <button
        type="button"
        className="agent-canvas-color-swatch agent-canvas-color-swatch--default"
        title={t(locale, 'agentCanvas.color.default')}
        aria-label={t(locale, 'agentCanvas.color.default')}
        aria-pressed={activeColor === null}
        onClick={() => onPick(null)}
      >
        {activeColor === null && <AppIcon name="check" aria-hidden="true" />}
      </button>
      {AGENT_CANVAS_COLOR_PRESETS.map((color) => (
        <button
          key={color}
          type="button"
          className="agent-canvas-color-swatch"
          style={{ backgroundColor: color }}
          title={color}
          aria-label={color}
          aria-pressed={activeColor === color}
          onClick={() => onPick(color)}
        >
          {activeColor === color && <AppIcon name="check" aria-hidden="true" />}
        </button>
      ))}
    </div>
  )
}
