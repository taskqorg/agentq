//! Register the stable launcher for the current OS user (agentq.md § 5).
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;

pub fn register(install: &Path) -> Result<(), String> {
    let install = std::path::absolute(install).map_err(|e| e.to_string())?;
    let exe = install.join(format!("agentq{}", std::env::consts::EXE_SUFFIX));
    if !exe.is_file() { return Err(format!("{}: installed launcher missing", exe.display())); }
    #[cfg(unix)]
    {
        let home = PathBuf::from(std::env::var_os("HOME").ok_or("HOME is unset")?);
        let zsh = std::env::var_os("ZDOTDIR").map_or_else(|| home.clone(), |p| home.join(p));
        register_unix(&home, &zsh, &exe)?;
    }
    #[cfg(windows)]
    windows(&install, "install")?;
    println!("command: registered; open a new terminal, then run `agentq status`");
    Ok(())
}

pub fn remove(install: &Path) -> Result<(), String> {
    let install = std::path::absolute(install).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        let home = PathBuf::from(std::env::var_os("HOME").ok_or("HOME is unset")?);
        remove_link(&home.join(".local/bin/agentq"), &install.join("agentq"))?;
    }
    #[cfg(windows)]
    windows(&install, "remove")?;
    Ok(())
}

#[cfg(unix)]
const PATH_BLOCK: &str = "\n# agentq: shared user command directory\ncase \":$PATH:\" in\n  *\":$HOME/.local/bin:\"*) ;;\n  *) export PATH=\"$HOME/.local/bin:$PATH\" ;;\nesac\n";

#[cfg(unix)]
fn register_unix(home: &Path, zsh: &Path, exe: &Path) -> Result<(), String> {
    let link = home.join(".local/bin/agentq");
    std::fs::create_dir_all(link.parent().unwrap()).map_err(|e| e.to_string())?;
    match std::fs::symlink_metadata(&link) {
        Ok(_) if std::fs::read_link(&link).ok().as_deref() == Some(exe) => {},
        Ok(_) => return Err(format!("{}: foreign command kept; resolve the conflict and retry", link.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => std::os::unix::fs::symlink(exe, &link).map_err(|e| e.to_string())?,
        Err(e) => return Err(e.to_string()),
    }
    // Creating .bash_profile would suppress a user's existing .profile.
    let login = [".bash_profile", ".bash_login", ".profile"].into_iter()
        .map(|p| home.join(p)).find(|p| p.exists()).unwrap_or_else(|| home.join(".profile"));
    for path in [login, home.join(".bashrc"), zsh.join(".zprofile"), zsh.join(".zshrc")] {
        let text = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        if !text.contains(PATH_BLOCK) {
            use std::io::Write;
            std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
            std::fs::OpenOptions::new().create(true).append(true).open(&path)
                .and_then(|mut f| f.write_all(PATH_BLOCK.as_bytes())).map_err(|e| format!("{}: {e}", path.display()))?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn remove_link(link: &Path, exe: &Path) -> Result<(), String> {
    if std::fs::read_link(link).ok().as_deref() == Some(exe) {
        std::fs::remove_file(link).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(windows)]
fn windows(install: &Path, action: &str) -> Result<(), String> {
    if install.to_string_lossy().contains(';') { return Err("install path contains a PATH separator".into()); }
    let script = format!("{}\n$key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Environment'); try {{ Set-AgentQPath $key $env:AGENTQ_COMMAND_DIR ($env:AGENTQ_COMMAND_ACTION -eq 'remove') }} finally {{ $key.Dispose() }}", include_str!("command_path.ps1"));
    let mut c = std::process::Command::new("powershell.exe");
    c.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", &script]).env("AGENTQ_COMMAND_DIR", install).env("AGENTQ_COMMAND_ACTION", action);
    let out = crate::host::output(&mut c).map_err(|e| format!("user PATH: {e}"))?;
    if out.status.success() { return Ok(()); }
    // A PowerShell error spans several lines; keep them all, or the exit status when there are none.
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Err(crate::host::failed(&c, &out, &format!("user PATH: {}", if err.is_empty() { crate::host::status(&out) } else { err })))
}

#[cfg(all(test, windows))]
mod windows_tests {
    /// Registry round trip in a disposable key: preserve raw expandable entries, avoid duplicates, remove only ours.
    #[test]
    fn user_path_registry() {
        let script = format!(r#"{}
$name = 'Software\AgentQTest-' + [Guid]::NewGuid().ToString()
$key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($name)
try {{
    $old = '%USERPROFILE%\foreign;C:\Other Tools'
    $key.SetValue('Path', $old, [Microsoft.Win32.RegistryValueKind]::ExpandString)
    Set-AgentQPath $key 'C:\AgentQ Test' $false
    Set-AgentQPath $key 'c:\agentq test\' $false
    $raw = $key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
    if ($raw -cne ($old + ';C:\AgentQ Test')) {{ throw "PATH lost or duplicated: $raw" }}
    if ($key.GetValueKind('Path') -ne 'ExpandString') {{ throw 'registry kind changed' }}
    Set-AgentQPath $key 'c:\AGENTQ TEST\' $true
    Set-AgentQPath $key 'c:\AGENTQ TEST\' $true
    if ($key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames) -cne $old) {{ throw 'foreign PATH changed' }}
}} finally {{ $key.Dispose(); [Microsoft.Win32.Registry]::CurrentUser.DeleteSubKeyTree($name) }}
"#, include_str!("command_path.ps1"));
        let out = std::process::Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command", &script]).output().unwrap();
        assert!(out.status.success(), "{out:?}");
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// A repaired command must run outside the install, survive repeat setup, and preserve foreign edits/removal.
    #[test]
    fn user_command_registration() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let home = std::env::temp_dir().join(format!("agentq-command-path-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let exe = home.join("custom install/agentq");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "#!/bin/sh\nprintf 'registered'\n").unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(home.join(".bash_profile"), "# user's login\n").unwrap();
        let zsh = home.join("zsh config");
        register_unix(&home, &zsh, &exe).unwrap();
        register_unix(&home, &zsh, &exe).unwrap();
        assert!(!home.join(".profile").exists());
        let profile = std::fs::read_to_string(home.join(".bash_profile")).unwrap();
        assert_eq!(profile, format!("# user's login\n{PATH_BLOCK}"));
        assert!(zsh.join(".zshrc").is_file());
        let out = std::process::Command::new("/bin/sh").args(["-c", ". \"$HOME/.bash_profile\"; agentq"])
            .env("HOME", &home).env("PATH", "/usr/bin:/bin").current_dir("/").output().unwrap();
        assert!(out.status.success(), "{out:?}");
        assert_eq!(out.stdout, b"registered");
        let link = home.join(".local/bin/agentq");
        remove_link(&link, &home.join("other/agentq")).unwrap();
        assert!(link.exists());
        remove_link(&link, &exe).unwrap();
        remove_link(&link, &exe).unwrap();
        assert_eq!(std::fs::read_to_string(home.join(".bash_profile")).unwrap(), profile);
        for foreign_link in [false, true] {
            if foreign_link { symlink("/foreign/agentq", &link).unwrap(); }
            else { std::fs::write(&link, "foreign command").unwrap(); }
            assert!(register_unix(&home, &zsh, &exe).unwrap_err().contains("foreign command kept"));
            remove_link(&link, &exe).unwrap();
            assert!(std::fs::symlink_metadata(&link).is_ok());
            std::fs::remove_file(&link).unwrap();
        }
        std::fs::remove_dir_all(home).unwrap();
    }
}
