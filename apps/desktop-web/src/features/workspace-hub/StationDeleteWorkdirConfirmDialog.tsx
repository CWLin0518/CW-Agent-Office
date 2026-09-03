import { useCallback, useEffect, useRef, type KeyboardEvent as ReactKeyboardEvent } from 'react'
import { t, type Locale } from '@shell/i18n/ui-locale'
import { AppIcon } from '@shell/ui/icons'
import { trapModalTabFocus } from '@/components/modal/modal-focus-trap'
import { createStationTerminalFrameFlushScheduler } from '../terminal/station-terminal-frame-flush-scheduler'
import { scheduleStationModalFocusFrame } from './station-modal-focus-frame'
import './StationDeleteWorkdirConfirmDialog.scss'

const STATION_DELETE_WORKDIR_DIALOG_FOCUS_FALLBACK_DELAY_MS = 48

interface StationDeleteWorkdirConfirmDialogProps {
  open: boolean
  locale: Locale
  stationName: string
  workdirPath: string
  onCancel: () => void
  /** Deletes the agent record and recursively removes `workdirPath`. Agent
   * deletion is already irreversible on its own, so this dialog exists only
   * as an extra confirmation step before the workdir is wiped too — there's
   * no separate "keep files" path. */
  onConfirm: () => void
}

export function StationDeleteWorkdirConfirmDialog({
  open,
  locale,
  stationName,
  workdirPath,
  onCancel,
  onConfirm,
}: StationDeleteWorkdirConfirmDialogProps) {
  const dialogRef = useRef<HTMLDivElement>(null)
  const cancelButtonRef = useRef<HTMLButtonElement>(null)

  useEffect(() => {
    if (open) {
      const focusFrame = scheduleStationModalFocusFrame({
        scheduler: createStationTerminalFrameFlushScheduler(window),
        fallbackDelayMs: STATION_DELETE_WORKDIR_DIALOG_FOCUS_FALLBACK_DELAY_MS,
        focus: () => {
          cancelButtonRef.current?.focus()
        },
      })
      return focusFrame.cancel
    }
  }, [open])

  const handleBackdropClick = useCallback(
    (e: React.MouseEvent) => {
      if (e.target === e.currentTarget) {
        onCancel()
      }
    },
    [onCancel],
  )

  const handleKeyDown = useCallback(
    (e: ReactKeyboardEvent) => {
      if (e.key !== 'Escape' && e.key !== 'Tab') {
        return
      }
      if (e.nativeEvent.isComposing) {
        return
      }
      if (e.key === 'Escape') {
        e.preventDefault()
        e.stopPropagation()
        onCancel()
        return
      }
      const dialog = dialogRef.current
      if (!dialog) {
        return
      }
      e.stopPropagation()
      trapModalTabFocus(e.nativeEvent, dialog)
    },
    [onCancel],
  )

  if (!open) return null

  return (
    <div
      className="station-delete-workdir-dialog-backdrop"
      onClick={handleBackdropClick}
      onKeyDown={handleKeyDown}
    >
      <div
        ref={dialogRef}
        className="station-delete-workdir-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="station-delete-workdir-dialog-title"
        aria-describedby="station-delete-workdir-dialog-message"
      >
        <div className="station-delete-workdir-dialog-header">
          <AppIcon name="info" className="station-delete-workdir-dialog-icon" aria-hidden="true" />
          <h3 id="station-delete-workdir-dialog-title" className="station-delete-workdir-dialog-title">
            {t(locale, 'station.deleteWorkdir.confirmTitle', { name: stationName })}
          </h3>
        </div>

        <div className="station-delete-workdir-dialog-body">
          <p id="station-delete-workdir-dialog-message" className="station-delete-workdir-dialog-message">
            {t(locale, 'station.deleteWorkdir.confirmMessage')}
          </p>
          <code className="station-delete-workdir-dialog-path">{workdirPath}</code>
        </div>

        <div className="station-delete-workdir-dialog-footer">
          <button
            ref={cancelButtonRef}
            type="button"
            className="station-delete-workdir-dialog-btn station-delete-workdir-dialog-btn-cancel"
            onClick={onCancel}
          >
            {t(locale, 'station.deleteWorkdir.cancel')}
          </button>
          <button
            type="button"
            className="station-delete-workdir-dialog-btn station-delete-workdir-dialog-btn-danger"
            onClick={onConfirm}
          >
            {t(locale, 'station.deleteWorkdir.deleteFiles')}
          </button>
        </div>
      </div>
    </div>
  )
}
