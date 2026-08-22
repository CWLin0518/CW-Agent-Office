import { desktopApi } from './desktop-api'

export interface DirectoryPickerOptions {
  defaultPath?: string | null
}

export async function pickDirectory(options?: DirectoryPickerOptions): Promise<string | null> {
  if (!desktopApi.isTauriRuntime()) {
    return null
  }

  try {
    return await desktopApi.systemPickDirectory(options?.defaultPath ?? null)
  } catch {
    return null
  }
}

export interface FilePickerOptions {
  defaultPath?: string | null
}

export async function pickFile(options?: FilePickerOptions): Promise<string | null> {
  if (!desktopApi.isTauriRuntime()) {
    return null
  }

  try {
    return await desktopApi.systemPickFile(options?.defaultPath ?? null)
  } catch {
    return null
  }
}
