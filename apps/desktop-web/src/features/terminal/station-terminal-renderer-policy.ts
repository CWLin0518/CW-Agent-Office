export interface StationTerminalRendererEnvironment {
  isMacOsWebKit: boolean
  isWindowsWebView: boolean
}

export function isWindowsWebViewEnvironment(userAgent: string): boolean {
  return /\bWindows\b/i.test(userAgent)
}

// Both WKWebView and WebView2 can keep a corrupt WebGL glyph texture atlas after
// compositor changes (window move, sleep, GPU switch), which shows up as garbled,
// overlapping or missing glyphs. xterm's DOM renderer avoids that GPU-only
// failure mode, so WebGL is only kept where it has not shown that problem.
export function shouldUseStationTerminalWebglRenderer(env: StationTerminalRendererEnvironment): boolean {
  return !env.isMacOsWebKit && !env.isWindowsWebView
}
