//! OS-specific helpers.

use std::path::Path;
use std::process::Command;

/// Opens the file in a text editor: VS Code if installed, otherwise the system default.
pub fn open_in_editor(path: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        if let Some(code) = find_in_path("code.cmd") {
            let _ = Command::new(code).arg(path).creation_flags(CREATE_NO_WINDOW).spawn();
        } else {
            let _ = Command::new("notepad.exe").arg(path).spawn();
        }
    }
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("open").arg("-t").arg(path).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if find_in_path("code").is_some() {
            let _ = Command::new("code").arg(path).spawn();
        } else {
            let _ = Command::new("xdg-open").arg(path).spawn();
        }
    }
}

/// Shows the file selected in the system file manager.
pub fn reveal(path: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // explorer needs the whole "/select,<path>" as one raw argument.
        let _ = Command::new("explorer.exe")
            .raw_arg(format!("/select,\"{}\"", path.display()))
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("open").arg("-R").arg(path).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(dir) = path.parent() {
            let _ = open::that_detached(dir);
        }
    }
}

/// Opens the OS page where the user picks default apps (Windows no longer lets apps
/// set themselves as default programmatically).
pub fn open_default_apps_settings() {
    #[cfg(windows)]
    let _ = open::that_detached("ms-settings:defaultapps");
}

fn find_in_path(exe: &str) -> Option<std::path::PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(exe))
        .find(|p| p.is_file())
}

/// Drag & drop from Explorer into an elevated process is blocked by Windows (UIPI), so
/// when started as administrator (e.g. from an elevated file manager) we relaunch
/// ourselves with the desktop shell's normal token. Returns true if the relaunch
/// succeeded and this process should exit.
#[cfg(windows)]
pub fn relaunch_unelevated(args: &[std::ffi::OsString]) -> bool {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{
        DuplicateTokenEx, GetTokenInformation, SecurityImpersonation, TOKEN_ADJUST_DEFAULT,
        TOKEN_ADJUST_SESSIONID, TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_ELEVATION,
        TOKEN_QUERY, TokenElevation, TokenPrimary,
    };
    use windows::Win32::System::Threading::{
        CREATE_PROCESS_LOGON_FLAGS, CreateProcessWithTokenW, GetCurrentProcess, OpenProcess,
        OpenProcessToken, PROCESS_CREATION_FLAGS, PROCESS_INFORMATION,
        PROCESS_QUERY_LIMITED_INFORMATION, STARTUPINFOW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetShellWindow, GetWindowThreadProcessId};
    use windows::core::{HSTRING, PCWSTR, PWSTR};

    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            if !self.0.is_invalid() {
                unsafe {
                    let _ = CloseHandle(self.0);
                }
            }
        }
    }

    unsafe {
        let mut token = Handle(HANDLE::default());
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token.0).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut len = 0;
        if GetTokenInformation(
            token.0,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        )
        .is_err()
            || elevation.TokenIsElevated == 0
        {
            return false;
        }

        let shell = GetShellWindow();
        if shell.is_invalid() {
            return false;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(shell, Some(&mut pid));
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let process = Handle(process);
        let mut shell_token = Handle(HANDLE::default());
        if OpenProcessToken(process.0, TOKEN_DUPLICATE, &mut shell_token.0).is_err() {
            return false;
        }
        let mut primary = Handle(HANDLE::default());
        if DuplicateTokenEx(
            shell_token.0,
            TOKEN_QUERY | TOKEN_ASSIGN_PRIMARY | TOKEN_DUPLICATE | TOKEN_ADJUST_DEFAULT | TOKEN_ADJUST_SESSIONID,
            None,
            SecurityImpersonation,
            TokenPrimary,
            &mut primary.0,
        )
        .is_err()
        {
            return false;
        }

        let Ok(exe) = std::env::current_exe() else { return false };
        let mut cmdline = quote(exe.as_os_str());
        cmdline.push_str(" --from-elevated");
        for arg in args {
            cmdline.push(' ');
            cmdline.push_str(&quote(arg));
        }
        let mut cmdline: Vec<u16> = cmdline.encode_utf16().chain(Some(0)).collect();
        let exe = HSTRING::from(exe.as_os_str());
        let cwd = std::env::current_dir().ok().map(|d| HSTRING::from(d.as_os_str()));

        let startup = STARTUPINFOW { cb: size_of::<STARTUPINFOW>() as u32, ..Default::default() };
        let mut info = PROCESS_INFORMATION::default();
        let ok = CreateProcessWithTokenW(
            primary.0,
            CREATE_PROCESS_LOGON_FLAGS(0),
            &exe,
            Some(PWSTR(cmdline.as_mut_ptr())),
            PROCESS_CREATION_FLAGS(0),
            None,
            cwd.as_ref().map_or(PCWSTR::null(), |d| PCWSTR(d.as_ptr())),
            &startup,
            &mut info,
        )
        .is_ok();
        if ok {
            let _ = CloseHandle(info.hProcess);
            let _ = CloseHandle(info.hThread);
        }
        ok
    }
}

#[cfg(windows)]
fn quote(arg: &std::ffi::OsStr) -> String {
    let s = arg.to_string_lossy();
    if !s.is_empty() && !s.contains([' ', '\t', '"']) {
        return s.into_owned();
    }
    // Windows command-line quoting: backslashes are literal unless they precede a quote.
    let mut out = String::from('"');
    let mut backslashes = 0;
    for c in s.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            c => {
                out.push_str(&"\\".repeat(backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}
