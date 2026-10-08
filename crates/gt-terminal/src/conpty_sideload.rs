//! Windows 10 ships an old in-box ConPTY that loses scrollback when a TUI
//! (Codex, Claude Code) inserts history above its inline viewport: scrolling
//! up shows the CLI's startup banner where the output should be. Newer
//! ConPTY builds (Microsoft.Windows.Console.ConPTY, MIT) fix this and run on
//! Windows 10 1809+.
//!
//! `portable-pty` already prefers a side-loaded `conpty.dll` over the
//! kernel32 exports, but looks it up by bare name. Loading our bundled copy by
//! full path first makes the loader hand that same module back for the bare
//! name, without touching the process-wide DLL search path. `conpty.dll`
//! starts the `OpenConsole.exe` sitting next to it.

use std::path::{Path, PathBuf};

pub const CONPTY_DLL: &str = "conpty.dll";
pub const OPEN_CONSOLE_EXE: &str = "OpenConsole.exe";

/// Pre-loads `dir/conpty.dll` when it and `OpenConsole.exe` are present.
/// `Ok` names the loaded DLL; `Err` says why the in-box ConPTY stays in use
/// (always `Err` on non-Windows targets). Must run before the first PTY is
/// spawned.
pub fn preload_sideloaded_conpty(dir: &Path) -> Result<PathBuf, String> {
    let dir = strip_verbatim_prefix(dir);
    let dll = dir.join(CONPTY_DLL);
    let result = if !dll.is_file() {
        Err(format!("missing {}", dll.display()))
    } else if !dir.join(OPEN_CONSOLE_EXE).is_file() {
        Err(format!("missing {}", dir.join(OPEN_CONSOLE_EXE).display()))
    } else {
        load_library(&dll).map(|()| dll.clone())
    };
    match &result {
        Ok(path) => tracing::info!(path = %path.display(), "using bundled ConPTY"),
        Err(reason) => tracing::warn!(reason, "bundled ConPTY not used; using in-box ConPTY"),
    }
    result
}

/// Tauri's resource dir comes back as `\\?\C:\...`; plain Win32 path
/// handling (and the loader's module lookup) is happier without the prefix.
fn strip_verbatim_prefix(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => path.to_path_buf(),
    }
}

#[cfg(windows)]
fn load_library(path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryW(file_name: *const u16) -> *mut std::ffi::c_void;
    }

    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<u16>>();
    // SAFETY: `wide` is a NUL-terminated UTF-16 path that outlives the call.
    // The module handle is intentionally never freed: it must stay loaded for
    // portable-pty's later bare-name lookup to resolve to it.
    let handle = unsafe { LoadLibraryW(wide.as_ptr()) };
    if handle.is_null() {
        Err(format!(
            "LoadLibraryW({}) failed: {}",
            path.display(),
            std::io::Error::last_os_error()
        ))
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn load_library(_path: &Path) -> Result<(), String> {
    Err("bundled ConPTY is Windows-only".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_bundle_is_a_no_op() {
        let dir = std::env::temp_dir().join("gt-terminal-conpty-missing");
        let reason = preload_sideloaded_conpty(&dir).expect_err("no bundle");
        assert!(reason.starts_with("missing"), "{reason}");
    }

    #[test]
    fn strips_verbatim_prefix_but_keeps_unc() {
        assert_eq!(
            strip_verbatim_prefix(Path::new(r"\\?\C:\app\resources")),
            PathBuf::from(r"C:\app\resources")
        );
        assert_eq!(
            strip_verbatim_prefix(Path::new(r"\\?\UNC\server\share")),
            PathBuf::from(r"\\?\UNC\server\share")
        );
    }
}
