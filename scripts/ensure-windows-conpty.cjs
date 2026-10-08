#!/usr/bin/env node
// Fetches the Microsoft.Windows.Console.ConPTY package (MIT, github.com/microsoft/terminal)
// and stages conpty.dll + OpenConsole.exe under src-tauri/resources/conpty, which
// tauri.windows.conf.json bundles. The in-box ConPTY on Windows 10 loses terminal
// scrollback under TUIs like Codex; see crates/gt-terminal/src/conpty_sideload.rs.
// Windows only; a no-op elsewhere.

const crypto = require('node:crypto')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { spawnSync } = require('node:child_process')

const CONPTY_VERSION = '1.24.260710001'
const CONPTY_SHA256 = '175640566a3b59c4b132070ee96c2c77e5ab7edd2e92732a5eb3610bbf63d90e'
const PACKAGE_URL = `https://api.nuget.org/v3-flatcontainer/microsoft.windows.console.conpty/${CONPTY_VERSION}/microsoft.windows.console.conpty.${CONPTY_VERSION}.nupkg`

const repoRoot = path.resolve(__dirname, '..')
const stageDir = path.join(repoRoot, 'apps', 'desktop-tauri', 'src-tauri', 'resources', 'conpty')
const versionMarker = path.join(stageDir, 'VERSION')

function resolveArch() {
  switch (process.env.GTO_CONPTY_ARCH || process.arch) {
    case 'arm64':
      return 'arm64'
    case 'ia32':
    case 'x86':
      return 'x86'
    default:
      return 'x64'
  }
}

function isStaged(arch) {
  try {
    return (
      fs.readFileSync(versionMarker, 'utf8').trim() === `${CONPTY_VERSION} ${arch}` &&
      fs.existsSync(path.join(stageDir, 'conpty.dll')) &&
      fs.existsSync(path.join(stageDir, 'OpenConsole.exe'))
    )
  } catch {
    return false
  }
}

async function download() {
  const response = await fetch(PACKAGE_URL)
  if (!response.ok) {
    throw new Error(`download failed: HTTP ${response.status}`)
  }
  const buffer = Buffer.from(await response.arrayBuffer())
  const digest = crypto.createHash('sha256').update(buffer).digest('hex')
  if (digest !== CONPTY_SHA256) {
    throw new Error(`checksum mismatch: expected ${CONPTY_SHA256}, got ${digest}`)
  }
  return buffer
}

function extract(buffer, arch) {
  const workDir = fs.mkdtempSync(path.join(os.tmpdir(), 'gtoffice-conpty-'))
  try {
    const zipPath = path.join(workDir, 'conpty.zip')
    const outDir = path.join(workDir, 'out')
    fs.writeFileSync(zipPath, buffer)
    const result = spawnSync(
      'powershell',
      [
        '-NoProfile',
        '-NonInteractive',
        '-Command',
        `Expand-Archive -LiteralPath '${zipPath}' -DestinationPath '${outDir}' -Force`,
      ],
      { stdio: 'inherit', shell: false },
    )
    if (result.status !== 0) {
      throw new Error('Expand-Archive failed')
    }
    fs.mkdirSync(stageDir, { recursive: true })
    fs.copyFileSync(
      path.join(outDir, 'runtimes', `win-${arch}`, 'native', 'conpty.dll'),
      path.join(stageDir, 'conpty.dll'),
    )
    fs.copyFileSync(
      path.join(outDir, 'build', 'native', 'runtimes', arch, 'OpenConsole.exe'),
      path.join(stageDir, 'OpenConsole.exe'),
    )
    fs.writeFileSync(versionMarker, `${CONPTY_VERSION} ${arch}\n`)
  } finally {
    fs.rmSync(workDir, { recursive: true, force: true })
  }
}

async function main() {
  if (process.platform !== 'win32') {
    return
  }
  const arch = resolveArch()
  if (isStaged(arch)) {
    return
  }
  console.log(`[GT Office] Fetching ConPTY ${CONPTY_VERSION} (${arch}) for the Windows terminal...`)
  extract(await download(), arch)
  console.log(`[GT Office] ConPTY staged at ${stageDir}`)
}

main().catch((error) => {
  console.error(`[GT Office] Failed to prepare bundled ConPTY: ${error.message}`)
  process.exit(1)
})
