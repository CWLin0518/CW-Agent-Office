import type { IWindowsPty } from '@xterm/xterm'

// ConPTY started reflowing its own buffer in this build; below it xterm must
// not reflow either, otherwise both sides rewrap and overwrite each other.
export const CONPTY_REFLOW_MIN_BUILD = 21376
const WINDOWS_11_FIRST_BUILD = 22000
const WINDOWS_10_LAST_BUILD = 19045

interface UserAgentDataLike {
  platform?: string
  getHighEntropyValues?: (hints: string[]) => Promise<{ platformVersion?: string }>
}

// Microsoft documents UA-CH platformVersion major >= 13 as Windows 11 and
// 1..10 as Windows 10; the exact build number is not exposed to the page.
export function resolveWindowsBuildNumberFromPlatformVersion(platformVersion: string): number | undefined {
  const major = Number.parseInt(platformVersion.split('.')[0] ?? '', 10)
  if (!Number.isFinite(major)) {
    return undefined
  }
  if (major >= 13) {
    return WINDOWS_11_FIRST_BUILD
  }
  if (major >= 1) {
    return WINDOWS_10_LAST_BUILD
  }
  return undefined
}

// Without windowsPty, xterm pulls scrollback back into the viewport when rows grow
// and reflows lines that ConPTY already wrapped. ConPTY then repaints on top of
// that content, which shows up as long agent output overwriting earlier lines.
export function buildStationTerminalWindowsPty(buildNumber?: number): IWindowsPty {
  return buildNumber === undefined ? { backend: 'conpty' } : { backend: 'conpty', buildNumber }
}

let windowsPtyDetection: Promise<IWindowsPty> | null = null

export function detectStationTerminalWindowsPty(userAgentData: UserAgentDataLike | undefined): Promise<IWindowsPty> {
  if (windowsPtyDetection) {
    return windowsPtyDetection
  }
  const getHighEntropyValues = userAgentData?.getHighEntropyValues
  if (!getHighEntropyValues) {
    windowsPtyDetection = Promise.resolve(buildStationTerminalWindowsPty())
    return windowsPtyDetection
  }
  windowsPtyDetection = getHighEntropyValues
    .call(userAgentData, ['platformVersion'])
    .then((values) =>
      buildStationTerminalWindowsPty(
        resolveWindowsBuildNumberFromPlatformVersion(values.platformVersion ?? ''),
      ),
    )
    .catch(() => buildStationTerminalWindowsPty())
  return windowsPtyDetection
}
