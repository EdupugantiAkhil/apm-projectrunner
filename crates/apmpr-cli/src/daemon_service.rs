use std::{
    env, fmt, fs, io,
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Command,
};

use sha2::{Digest, Sha256};

#[derive(Debug)]
pub struct InstalledService {
    pub definition: PathBuf,
    pub manager: &'static str,
}

#[derive(Debug)]
pub enum ServiceError {
    Io(io::Error),
    Message(String),
}

impl fmt::Display for ServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(formatter),
            Self::Message(message) => message.fmt(formatter),
        }
    }
}

impl std::error::Error for ServiceError {}

impl From<io::Error> for ServiceError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

pub fn install(project_root: &Path, executable: &Path) -> Result<InstalledService, ServiceError> {
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| ServiceError::Message("HOME is not set to an absolute path".into()))?;
    let path = env::var("PATH").map_err(|_| {
        ServiceError::Message("PATH is not set to valid UTF-8 for the daemon service".into())
    })?;
    let log = prepare_log(project_root)?;

    #[cfg(target_os = "macos")]
    {
        install_launchd(&home, project_root, executable, &log, &path)
    }
    #[cfg(target_os = "linux")]
    {
        let config_home = env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| home.join(".config"));
        install_systemd(&config_home, project_root, executable, &log, &path)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (home, executable, log, path);
        Err(ServiceError::Message(
            "daemon service installation is supported only on macOS and Linux".into(),
        ))
    }
}

fn prepare_log(project_root: &Path) -> Result<PathBuf, ServiceError> {
    let state = project_root.join(".apmpr");
    fs::create_dir_all(&state)?;
    let log = state.join("daemon.log");
    if let Ok(metadata) = fs::symlink_metadata(&log) {
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err(ServiceError::Message(format!(
                "refusing daemon log path that is not a regular file: {}",
                log.display()
            )));
        }
    }
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&log)?;
    fs::set_permissions(&log, fs::Permissions::from_mode(0o600))?;
    Ok(log)
}

#[cfg(target_os = "linux")]
fn install_systemd(
    config_home: &Path,
    project_root: &Path,
    executable: &Path,
    log: &Path,
    path: &str,
) -> Result<InstalledService, ServiceError> {
    let unit_name = format!("apmpr-{}.service", service_key(project_root));
    let definition = config_home.join("systemd/user").join(&unit_name);
    let contents = render_systemd(project_root, executable, log, path)?;
    atomic_write(&definition, contents.as_bytes())?;

    run(
        Command::new("systemctl").args(["--user", "daemon-reload"]),
        "reload systemd user units",
    )?;
    run(
        Command::new("systemctl").args(["--user", "enable", "--now", &unit_name]),
        "enable and start the daemon service",
    )?;
    Ok(InstalledService {
        definition,
        manager: "systemd user service",
    })
}

#[cfg(target_os = "macos")]
fn install_launchd(
    home: &Path,
    project_root: &Path,
    executable: &Path,
    log: &Path,
    path: &str,
) -> Result<InstalledService, ServiceError> {
    let label = format!("dev.apmpr.daemon.{}", service_key(project_root));
    let definition = home
        .join("Library/LaunchAgents")
        .join(format!("{label}.plist"));
    let contents = render_launchd(&label, project_root, executable, log, path)?;
    atomic_write(&definition, contents.as_bytes())?;

    let uid = run_output(
        Command::new("id").arg("-u"),
        "determine the current user id",
    )?;
    if uid.is_empty() || !uid.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ServiceError::Message(format!(
            "`id -u` returned an invalid user id: {uid:?}"
        )));
    }
    let domain = format!("gui/{uid}");
    let _ = Command::new("launchctl")
        .args(["bootout", &domain])
        .arg(&definition)
        .status();
    run(
        Command::new("launchctl")
            .args(["bootstrap", &domain])
            .arg(&definition),
        "register the daemon LaunchAgent",
    )?;
    run(
        Command::new("launchctl").args(["kickstart", "-k", &format!("{domain}/{label}")]),
        "start the daemon LaunchAgent",
    )?;
    Ok(InstalledService {
        definition,
        manager: "launchd LaunchAgent",
    })
}

fn service_key(project_root: &Path) -> String {
    let name = project_root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("project")
        .chars()
        .take(48)
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    let name = name.trim_matches('-');
    let name = if name.is_empty() { "project" } else { name };
    let digest = Sha256::digest(project_root.as_os_str().as_encoded_bytes());
    format!(
        "{name}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        digest[0], digest[1], digest[2], digest[3], digest[4], digest[5]
    )
}

#[cfg(any(target_os = "linux", test))]
fn render_systemd(
    project_root: &Path,
    executable: &Path,
    log: &Path,
    path: &str,
) -> Result<String, ServiceError> {
    let project = systemd_quote(path_text(project_root)?);
    let executable = systemd_quote(path_text(executable)?);
    let log = systemd_quote(&format!("append:{}", path_text(log)?));
    let path = systemd_quote(&format!("PATH={path}"));
    Ok(format!(
        "[Unit]\nDescription=APM ProjectRunner daemon for {project}\n\n[Service]\nType=simple\nWorkingDirectory={project}\nExecStart={executable} daemon run\nEnvironment={path}\nRestart=on-failure\nRestartSec=2\nStandardOutput={log}\nStandardError={log}\n\n[Install]\nWantedBy=default.target\n"
    ))
}

#[cfg(any(target_os = "macos", test))]
fn render_launchd(
    label: &str,
    project_root: &Path,
    executable: &Path,
    log: &Path,
    path: &str,
) -> Result<String, ServiceError> {
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n  <key>Label</key>\n  <string>{}</string>\n  <key>ProgramArguments</key>\n  <array>\n    <string>{}</string>\n    <string>daemon</string>\n    <string>run</string>\n  </array>\n  <key>WorkingDirectory</key>\n  <string>{}</string>\n  <key>EnvironmentVariables</key>\n  <dict><key>PATH</key><string>{}</string></dict>\n  <key>RunAtLoad</key>\n  <true/>\n  <key>KeepAlive</key>\n  <dict><key>SuccessfulExit</key><false/></dict>\n  <key>ThrottleInterval</key>\n  <integer>2</integer>\n  <key>StandardOutPath</key>\n  <string>{}</string>\n  <key>StandardErrorPath</key>\n  <string>{}</string>\n</dict>\n</plist>\n",
        xml_escape(label),
        xml_escape(path_text(executable)?),
        xml_escape(path_text(project_root)?),
        xml_escape(path),
        xml_escape(path_text(log)?),
        xml_escape(path_text(log)?),
    ))
}

fn path_text(path: &Path) -> Result<&str, ServiceError> {
    path.to_str().ok_or_else(|| {
        ServiceError::Message(format!(
            "service definitions require a UTF-8 path: {}",
            path.display()
        ))
    })
}

#[cfg(any(target_os = "linux", test))]
fn systemd_quote(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
        .replace('%', "%%");
    format!("\"{escaped}\"")
}

#[cfg(any(target_os = "macos", test))]
fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), ServiceError> {
    let parent = path.parent().ok_or_else(|| {
        ServiceError::Message(format!("service path has no parent: {}", path.display()))
    })?;
    fs::create_dir_all(parent)?;
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err(ServiceError::Message(format!(
                "refusing service definition that is not a regular file: {}",
                path.display()
            )));
        }
    }
    let temporary = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("apmpr-service"),
        std::process::id()
    ));
    let result = (|| -> io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(contents)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(ServiceError::Io)
}

fn run(command: &mut Command, action: &str) -> Result<(), ServiceError> {
    let debug = format!("{command:?}");
    let status = command.status().map_err(|error| {
        ServiceError::Message(format!("could not {action} with {debug}: {error}"))
    })?;
    if status.success() {
        Ok(())
    } else {
        Err(ServiceError::Message(format!(
            "could not {action}: {debug} exited with {status}"
        )))
    }
}

#[cfg(target_os = "macos")]
fn run_output(command: &mut Command, action: &str) -> Result<String, ServiceError> {
    let debug = format!("{command:?}");
    let output = command.output().map_err(|error| {
        ServiceError::Message(format!("could not {action} with {debug}: {error}"))
    })?;
    if !output.status.success() {
        return Err(ServiceError::Message(format!(
            "could not {action}: {debug} exited with {}",
            output.status
        )));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_owned())
        .map_err(|_| ServiceError::Message(format!("{debug} returned non-UTF-8 output")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn systemd_unit_quotes_paths_and_percent_specifiers() {
        let unit = render_systemd(
            Path::new("/tmp/My Project%20"),
            Path::new("/opt/Switch Yard/bin/apmpr"),
            Path::new("/tmp/My Project%20/.apmpr/daemon.log"),
            "/opt/Switch Yard/bin:/usr/bin",
        )
        .unwrap();
        assert!(unit.contains("WorkingDirectory=\"/tmp/My Project%%20\""));
        assert!(unit.contains("ExecStart=\"/opt/Switch Yard/bin/apmpr\" daemon run"));
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains("Environment=\"PATH=/opt/Switch Yard/bin:/usr/bin\""));
        assert!(unit.contains("StandardError=\"append:/tmp/My Project%%20/.apmpr/daemon.log\""));
    }

    #[test]
    fn launchd_plist_escapes_paths_and_has_service_posture() {
        let plist = render_launchd(
            "dev.apmpr.daemon.demo",
            Path::new("/tmp/A&B"),
            Path::new("/opt/<apmpr>"),
            Path::new("/tmp/A&B/.apmpr/daemon.log"),
            "/opt/A&B/bin:/usr/bin",
        )
        .unwrap();
        assert!(plist.contains("<string>/tmp/A&amp;B</string>"));
        assert!(plist.contains("<string>/opt/&lt;apmpr&gt;</string>"));
        assert!(plist.contains("<key>PATH</key><string>/opt/A&amp;B/bin:/usr/bin</string>"));
        assert!(plist.contains("<key>RunAtLoad</key>\n  <true/>"));
        assert!(plist.contains("<key>KeepAlive</key>"));
    }

    #[test]
    fn service_keys_are_readable_stable_and_project_specific() {
        let first = service_key(Path::new("/tmp/My Project"));
        assert!(first.starts_with("my-project-"));
        assert_eq!(first, service_key(Path::new("/tmp/My Project")));
        assert_ne!(first, service_key(Path::new("/other/My Project")));
    }

    #[test]
    fn atomic_write_replaces_regular_files_but_refuses_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("service");
        atomic_write(&path, b"one").unwrap();
        atomic_write(&path, b"two").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"two");

        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(temp.path().join("target"), &path).unwrap();
        assert!(atomic_write(&path, b"no").is_err());
    }

    #[test]
    fn daemon_log_is_owner_only_and_refuses_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        let log = prepare_log(temp.path()).unwrap();
        assert_eq!(
            fs::metadata(&log).unwrap().permissions().mode() & 0o777,
            0o600
        );

        fs::remove_file(&log).unwrap();
        std::os::unix::fs::symlink(temp.path().join("target"), &log).unwrap();
        assert!(prepare_log(temp.path()).is_err());
    }
}
