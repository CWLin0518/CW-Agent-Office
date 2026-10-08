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

use std::path::Path;

pub const CONPTY_DLL: &str = "conpty.dll";
pub const OPEN_CONSOLE_EXE: &str = "OpenConsole.exe";

/// Pre-loads `dir/conpty.dll` when it and `OpenConsole.exe` are present.
/// Returns whether the bundled ConPTY is now in use. Must run before the first
/// PTY is spawned; a no-op (returning `false`) on non-Windows targets.
pub fn preload_sideloaded_conpty(dir: &Path) -> bool {
    let dll = dir.join(CONPTY_DLL);
    if !dll.is_file() || !dir.join(OPEN_CONSOLE_EXE).is_file() {
        tracing::info!(dir = %dir.display(), "bundled ConPTY not found; using in-box ConPTY");
        return false;
    }
    let loaded = load_library(&dll);
    if loaded {
        tracing::info!(path = %dll.display(), "using bundled ConPTY");
    } else {
        tracing::warn!(path = %dll.display(), "failed to load bundled ConPTY; using in-box ConPTY");
    }
    loaded
}

#[cfg(windows)]
fn load_library(path: &Path) -> bool {
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
    !handle.is_null()
}

#[cfg(not(windows))]
fn load_library(_path: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_bundle_is_a_no_op() {
        let dir = std::env::temp_dir().join("gt-terminal-conpty-missing");
        assert!(!preload_sideloaded_conpty(&dir));
    }
}
