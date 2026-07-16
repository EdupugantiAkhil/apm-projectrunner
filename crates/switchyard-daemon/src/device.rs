//! Safe, shell-free SSH connectivity checks shared by the daemon and CLI.

use std::{ffi::OsString, fmt, io, path::Path, process::Command};

use switchyard_state::{DeviceCheckStatus, RegisteredDevice};

#[derive(Debug)]
pub enum DeviceCheckError {
    SshUnavailable(io::Error),
    Io(io::Error),
}

impl DeviceCheckError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::SshUnavailable(_) => "ssh_unavailable",
            Self::Io(_) => "ssh_check_io",
        }
    }
}

impl fmt::Display for DeviceCheckError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SshUnavailable(_) => write!(formatter, "system `ssh` binary is unavailable"),
            Self::Io(error) => write!(formatter, "failed to run SSH connectivity check: {error}"),
        }
    }
}

impl std::error::Error for DeviceCheckError {}

pub fn ssh_arguments(device: &RegisteredDevice) -> Vec<OsString> {
    let mut arguments = vec![
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "ConnectTimeout=5".into(),
        "-o".into(),
        "StrictHostKeyChecking=accept-new".into(),
    ];
    if let Some(identity) = &device.identity_file {
        arguments.push("-i".into());
        arguments.push(identity.as_os_str().to_owned());
    }
    arguments.extend([
        "-p".into(),
        device.port.to_string().into(),
        format!("{}@{}", device.user, device.host).into(),
        "true".into(),
    ]);
    arguments
}

pub fn check(device: &RegisteredDevice) -> Result<(DeviceCheckStatus, String), DeviceCheckError> {
    check_with_program(Path::new("ssh"), device)
}

pub fn check_with_program(
    program: &Path,
    device: &RegisteredDevice,
) -> Result<(DeviceCheckStatus, String), DeviceCheckError> {
    let output = Command::new(program)
        .args(ssh_arguments(device))
        .output()
        .map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                DeviceCheckError::SshUnavailable(error)
            } else {
                DeviceCheckError::Io(error)
            }
        })?;
    Ok(map_output(output.status.code(), &output.stderr))
}

pub fn map_output(exit_code: Option<i32>, stderr: &[u8]) -> (DeviceCheckStatus, String) {
    if exit_code == Some(0) {
        return (DeviceCheckStatus::Ok, "SSH connection succeeded".into());
    }
    let detail = String::from_utf8_lossy(stderr)
        .trim()
        .chars()
        .take(2_000)
        .collect::<String>();
    let lower = detail.to_ascii_lowercase();
    let status = if lower.contains("permission denied")
        || lower.contains("authentication failed")
        || lower.contains("too many authentication failures")
    {
        DeviceCheckStatus::AuthFailed
    } else {
        DeviceCheckStatus::Unreachable
    };
    let outcome = exit_code.map_or_else(
        || "terminated by signal".into(),
        |code| format!("exited with status {code}"),
    );
    let detail = if detail.is_empty() {
        format!("SSH {outcome} without an error message")
    } else {
        format!("SSH {outcome}: {detail}")
    };
    (status, detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn device() -> RegisteredDevice {
        RegisteredDevice {
            name: "build".into(),
            host: "host.test".into(),
            port: 2222,
            user: "dev".into(),
            identity_file: Some(PathBuf::from("keys/id_ed25519")),
            created_at: 1,
            last_checked_at: None,
            last_check_status: DeviceCheckStatus::Never,
            last_check_detail: None,
        }
    }

    #[test]
    fn constructs_shell_free_arguments_and_maps_results() {
        assert_eq!(
            ssh_arguments(&device()),
            vec![
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=5",
                "-o",
                "StrictHostKeyChecking=accept-new",
                "-i",
                "keys/id_ed25519",
                "-p",
                "2222",
                "dev@host.test",
                "true"
            ]
            .into_iter()
            .map(OsString::from)
            .collect::<Vec<_>>()
        );
        assert_eq!(map_output(Some(0), b"").0, DeviceCheckStatus::Ok);
        assert_eq!(
            map_output(Some(255), b"Permission denied (publickey).").0,
            DeviceCheckStatus::AuthFailed
        );
        assert_eq!(
            map_output(Some(255), b"Connection timed out").0,
            DeviceCheckStatus::Unreachable
        );
    }
}
