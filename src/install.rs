//! Per-user Windows install.
//!
//! The release download stays `token-dock.exe`. When that file is launched from
//! anywhere else, it is copied to `%LOCALAPPDATA%\TokenDock\token-dock.exe`,
//! a Start menu shortcut is created, and the installed copy is started.
//! Settings remain in `%LOCALAPPDATA%\token-dock`.
//!
//! This process exits after it starts the installed copy. It does not take the
//! single-instance lock, so the short-lived installer cannot block that copy.

use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

const INSTALL_DIRECTORY: &str = "TokenDock";
const INSTALL_FILE: &str = "token-dock.exe";
const SHORTCUT_FILE: &str = "TokenDock.lnk";

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Action {
    /// This process is already the installed executable.
    Run,
    /// Copy this executable into place, create the shortcut, then start it.
    Install,
    /// The installed executable is the same program. Start it.
    StartInstalled,
    /// The installed executable is running and differs. Replace it after exit.
    ReplaceAfterExit { process_id: u32 },
}

pub(crate) fn install_exe(local_app_data: &Path) -> PathBuf {
    local_app_data.join(INSTALL_DIRECTORY).join(INSTALL_FILE)
}

pub(crate) fn start_menu_shortcut(roaming_app_data: &Path) -> PathBuf {
    roaming_app_data
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs")
        .join(SHORTCUT_FILE)
}

/// `running_pid` is the installed executable's process, when one is running.
/// `destination_locked` means that file cannot be replaced in place.
pub(crate) fn decide(
    current: &Path,
    installed: &Path,
    running_pid: Option<u32>,
    destination_locked: bool,
) -> Action {
    if same_location(current, installed) {
        return Action::Run;
    }
    if !installed.is_file() {
        return Action::Install;
    }
    if same_contents(current, installed) {
        return Action::StartInstalled;
    }
    if let Some(process_id) = running_pid {
        return Action::ReplaceAfterExit { process_id };
    }
    if destination_locked {
        return Action::Run;
    }
    Action::Install
}

pub(crate) fn same_location(left: &Path, right: &Path) -> bool {
    path_key(&plain(left)) == path_key(&plain(right))
}

fn plain(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn path_key(path: &Path) -> String {
    let text = path.to_string_lossy().replace('/', "\\");
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    text.to_ascii_lowercase()
}

fn same_contents(left: &Path, right: &Path) -> bool {
    let Ok(mut left_file) = File::open(left) else {
        return false;
    };
    let Ok(mut right_file) = File::open(right) else {
        return false;
    };
    let Ok(left_len) = left_file.metadata().map(|meta| meta.len()) else {
        return false;
    };
    let Ok(right_len) = right_file.metadata().map(|meta| meta.len()) else {
        return false;
    };
    if left_len != right_len {
        return false;
    }
    let mut left_buf = [0_u8; 65_536];
    let mut right_buf = [0_u8; 65_536];
    loop {
        let count = match left_file.read(&mut left_buf) {
            Ok(0) => return true,
            Ok(count) => count,
            Err(_) => return false,
        };
        if right_file.read_exact(&mut right_buf[..count]).is_err()
            || left_buf[..count] != right_buf[..count]
        {
            return false;
        }
    }
}

pub(crate) fn copy_into_place(source: &Path, dest: &Path) -> std::io::Result<()> {
    if same_location(source, dest) {
        return Ok(());
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let partial = sibling(dest, ".partial");
    let backup = sibling(dest, ".previous");
    fs::copy(source, &partial)?;
    if dest.exists() {
        let _ = fs::remove_file(&backup);
        if let Err(error) = fs::rename(dest, &backup) {
            let _ = fs::remove_file(&partial);
            return Err(error);
        }
    }
    if let Err(error) = fs::rename(&partial, dest) {
        if backup.is_file() {
            let _ = fs::rename(&backup, dest);
        }
        let _ = fs::remove_file(&partial);
        return Err(error);
    }
    let _ = fs::remove_file(&backup);
    Ok(())
}

fn stage_replacement(source: &Path, installed: &Path) -> std::io::Result<PathBuf> {
    let stage = installed.with_file_name("token-dock.update.exe");
    copy_into_place(source, &stage)?;
    Ok(stage)
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path
        .file_name()
        .unwrap_or(std::ffi::OsStr::new(INSTALL_FILE))
        .to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

#[cfg(windows)]
pub fn install_and_handoff() {
    let (Some(local), Some(roaming)) = (dirs::data_local_dir(), dirs::data_dir()) else {
        return;
    };
    let installed = install_exe(&local);
    let Ok(current) = std::env::current_exe() else {
        return;
    };
    let shortcut = start_menu_shortcut(&roaming);
    let check_running = !same_location(&current, &installed)
        && installed.is_file()
        && !same_contents(&current, &installed);
    let running_pid = if check_running {
        process_id(&installed)
    } else {
        None
    };
    let destination_locked =
        check_running && (running_pid.is_some() || sharing_violation(&installed));
    match decide(&current, &installed, running_pid, destination_locked) {
        Action::Run => {}
        Action::Install => {
            if let Err(error) = copy_into_place(&current, &installed) {
                let pid = running_pid.or_else(|| process_id(&installed));
                if error.raw_os_error() == Some(SHARING_VIOLATION)
                    && let Some(pid) = pid
                    && schedule_replace(pid, &current, &installed)
                {
                    std::process::exit(0);
                }
                return;
            }
            let _ = write_shortcut(&shortcut, &installed);
            if start_detached(&installed).is_ok() {
                std::process::exit(0);
            }
        }
        Action::StartInstalled => {
            if start_detached(&installed).is_ok() {
                std::process::exit(0);
            }
        }
        Action::ReplaceAfterExit { process_id } => {
            if schedule_replace(process_id, &current, &installed) {
                std::process::exit(0);
            }
        }
    }
}

#[cfg(windows)]
fn schedule_replace(process_id: u32, current: &Path, installed: &Path) -> bool {
    let Ok(stage) = stage_replacement(current, installed) else {
        return false;
    };
    crate::update::replace_after_exit(process_id, &stage, installed)
}

/// Windows ERROR_SHARING_VIOLATION. A running executable cannot be opened for writing.
#[cfg(windows)]
const SHARING_VIOLATION: i32 = 32;

#[cfg(windows)]
fn sharing_violation(path: &Path) -> bool {
    match std::fs::OpenOptions::new().write(true).open(path) {
        Ok(_) => false,
        Err(error) => error.raw_os_error() == Some(SHARING_VIOLATION),
    }
}

#[cfg(windows)]
fn process_id(exe: &Path) -> Option<u32> {
    use std::{
        os::windows::process::CommandExt,
        process::{Command, Stdio},
    };
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-WindowStyle",
            "Hidden",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            "$want = [IO.Path]::GetFullPath($env:TOKEN_DOCK_INSTALL); Get-CimInstance Win32_Process -Filter \"Name = 'token-dock.exe'\" | Where-Object { $_.ExecutablePath -and ([IO.Path]::GetFullPath($_.ExecutablePath) -ieq $want) } | Select-Object -First 1 -ExpandProperty ProcessId",
        ])
        .env("TOKEN_DOCK_INSTALL", exe)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    let text = String::from_utf8(output.stdout).ok()?;
    text.split_whitespace().next()?.parse().ok()
}

#[cfg(windows)]
fn write_shortcut(shortcut: &Path, target: &Path) -> std::io::Result<()> {
    use std::{
        os::windows::process::CommandExt,
        process::{Command, Stdio},
    };
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    if let Some(parent) = shortcut.parent() {
        fs::create_dir_all(parent)?;
    }
    let status = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-WindowStyle",
            "Hidden",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            "$ErrorActionPreference = 'Stop'; $shell = New-Object -ComObject WScript.Shell; $link = $shell.CreateShortcut($env:TOKEN_DOCK_SHORTCUT); $link.TargetPath = $env:TOKEN_DOCK_TARGET; $link.WorkingDirectory = Split-Path -LiteralPath $env:TOKEN_DOCK_TARGET; $link.Save()",
        ])
        .env("TOKEN_DOCK_SHORTCUT", shortcut)
        .env("TOKEN_DOCK_TARGET", target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(
            "The Start menu shortcut was not saved.",
        ))
    }
}

#[cfg(windows)]
fn start_detached(exe: &Path) -> std::io::Result<()> {
    use std::process::{Command, Stdio};
    let mut command = Command::new(exe);
    if let Some(parent) = exe.parent() {
        command.current_dir(parent);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_paths_stay_out_of_the_settings_directory() {
        let local = Path::new(r"C:\Users\Ada\AppData\Local");
        let roaming = Path::new(r"C:\Users\Ada\AppData\Roaming");
        assert_eq!(
            path_key(&install_exe(local)),
            r"c:\users\ada\appdata\local\tokendock\token-dock.exe"
        );
        assert_eq!(
            path_key(&start_menu_shortcut(roaming)),
            r"c:\users\ada\appdata\roaming\microsoft\windows\start menu\programs\tokendock.lnk"
        );
        assert_ne!(
            install_exe(local)
                .parent()
                .and_then(|path| path.file_name()),
            Some(std::ffi::OsStr::new("token-dock"))
        );
    }

    #[test]
    fn handoff_logic_does_not_write_a_start_menu_shortcut() {
        let root = tempfile::tempdir().unwrap();
        let current = root.path().join("Downloads").join("token-dock.exe");
        fs::create_dir_all(current.parent().unwrap()).unwrap();
        fs::write(&current, b"version-two").unwrap();
        let installed = install_exe(&root.path().join("Local"));
        let shortcut = start_menu_shortcut(&root.path().join("Roaming"));

        assert_eq!(decide(&current, &installed, None, false), Action::Install);
        copy_into_place(&current, &installed).unwrap();
        assert_eq!(fs::read(&installed).unwrap(), b"version-two");

        assert_eq!(
            decide(&current, &installed, Some(7), true),
            Action::StartInstalled
        );
        fs::write(&current, b"version-two-plus").unwrap();
        assert!(!same_contents(&current, &installed));
        fs::write(&current, b"version-xxx").unwrap();
        assert!(!same_contents(&current, &installed));
        assert_eq!(
            decide(&current, &installed, Some(7), true),
            Action::ReplaceAfterExit { process_id: 7 }
        );
        let stage = stage_replacement(&current, &installed).unwrap();
        assert_eq!(stage.file_name().unwrap(), "token-dock.update.exe");
        assert_eq!(fs::read(&stage).unwrap(), b"version-xxx");
        assert_eq!(fs::read(&installed).unwrap(), b"version-two");
        assert_eq!(decide(&current, &installed, None, true), Action::Run);
        assert_eq!(fs::read(&installed).unwrap(), b"version-two");

        assert_eq!(decide(&current, &installed, None, false), Action::Install);
        copy_into_place(&current, &installed).unwrap();
        assert_eq!(fs::read(&installed).unwrap(), b"version-xxx");
        assert_eq!(decide(&installed, &installed, Some(7), true), Action::Run);
        let mut big = vec![1_u8; 70_000];
        fs::write(&current, &big).unwrap();
        big[69_999] = 2;
        fs::write(&installed, &big).unwrap();
        assert!(!same_contents(&current, &installed));
        big[69_999] = 1;
        fs::write(&installed, &big).unwrap();
        assert!(same_contents(&current, &installed));

        let extended = Path::new(r"\\?\C:\Users\Ada\AppData\Local\TokenDock\token-dock.exe");
        let install = Path::new(r"C:\Users\Ada\AppData\Local\TokenDock\token-dock.exe");
        assert!(same_location(extended, install));
        assert_eq!(decide(extended, install, Some(7), true), Action::Run);
        assert!(!shortcut.exists());
        assert!(!shortcut.parent().unwrap().exists());
    }
}
