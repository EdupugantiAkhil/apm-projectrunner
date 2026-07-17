use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    io,
    process::Command,
};

use switchyard_state::{DeviceCheckStatus, RegisteredDevice};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckOutput {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

pub trait DeviceCheckExecutor {
    fn run(
        &self,
        program: &OsStr,
        arguments: &[OsString],
        environment: &BTreeMap<OsString, OsString>,
    ) -> io::Result<CheckOutput>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemDeviceCheckExecutor;

impl DeviceCheckExecutor for SystemDeviceCheckExecutor {
    fn run(
        &self,
        program: &OsStr,
        arguments: &[OsString],
        environment: &BTreeMap<OsString, OsString>,
    ) -> io::Result<CheckOutput> {
        let output = Command::new(program)
            .args(arguments)
            .envs(environment)
            .output()?;
        Ok(CheckOutput {
            success: output.status.success(),
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

pub fn check_device_eligibility(device: &RegisteredDevice) -> (DeviceCheckStatus, String) {
    check_device_eligibility_with(&SystemDeviceCheckExecutor, device)
}

pub fn check_device_eligibility_with<E: DeviceCheckExecutor>(
    executor: &E,
    device: &RegisteredDevice,
) -> (DeviceCheckStatus, String) {
    let ssh = match executor.run(OsStr::new("ssh"), &ssh_arguments(device), &BTreeMap::new()) {
        Ok(output) => output,
        Err(error) => {
            return (
                DeviceCheckStatus::Unreachable,
                format!("no docker over SSH: SSH probe could not start: {error}"),
            );
        }
    };
    if !ssh.success {
        let reason = output_reason(&ssh);
        let status = if reason.to_ascii_lowercase().contains("permission denied")
            || reason
                .to_ascii_lowercase()
                .contains("authentication failed")
        {
            DeviceCheckStatus::AuthFailed
        } else {
            DeviceCheckStatus::Unreachable
        };
        return (status, format!("no docker over SSH: SSH failed: {reason}"));
    }

    let docker = match executor.run(
        OsStr::new("docker"),
        &[
            "version".into(),
            "--format".into(),
            "{{.Server.Version}}".into(),
        ],
        &docker_environment(device),
    ) {
        Ok(output) => output,
        Err(error) => {
            return (
                DeviceCheckStatus::Ineligible,
                format!("no docker over SSH: Docker client could not start: {error}"),
            );
        }
    };
    let version = docker.stdout.trim();
    if docker.success && !version.is_empty() {
        return (
            DeviceCheckStatus::Eligible,
            format!("eligible for remote container execution (docker {version})"),
        );
    }
    let reason = if docker.success {
        "Docker returned no server version".into()
    } else {
        output_reason(&docker)
    };
    (
        DeviceCheckStatus::Ineligible,
        format!("no docker over SSH: {reason}"),
    )
}

pub fn eligibility_label(device: &RegisteredDevice) -> String {
    match device.last_check_status {
        DeviceCheckStatus::Eligible => device
            .last_check_detail
            .as_deref()
            .and_then(|detail| detail.strip_prefix("eligible for remote container execution "))
            .map_or_else(
                || "eligible".into(),
                |version| format!("eligible {version}"),
            ),
        DeviceCheckStatus::Ineligible => device
            .last_check_detail
            .clone()
            .unwrap_or_else(|| "no docker over SSH: eligibility check failed".into()),
        DeviceCheckStatus::Unreachable | DeviceCheckStatus::AuthFailed => device
            .last_check_detail
            .clone()
            .unwrap_or_else(|| "no docker over SSH: SSH failed".into()),
        DeviceCheckStatus::Never | DeviceCheckStatus::Ok => "unchecked".into(),
    }
}

fn ssh_arguments(device: &RegisteredDevice) -> Vec<OsString> {
    let mut arguments = vec![
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "ConnectTimeout=5".into(),
        "-o".into(),
        "StrictHostKeyChecking=accept-new".into(),
    ];
    if let Some(identity) = &device.identity_file {
        arguments.extend(["-i".into(), identity.as_os_str().to_owned()]);
    }
    arguments.extend([
        "-p".into(),
        device.port.to_string().into(),
        format!("{}@{}", device.user, device.host).into(),
        "true".into(),
    ]);
    arguments
}

fn docker_environment(device: &RegisteredDevice) -> BTreeMap<OsString, OsString> {
    let mut environment = BTreeMap::from([(
        "DOCKER_HOST".into(),
        format!("ssh://{}@{}:{}", device.user, device.host, device.port).into(),
    )]);
    let mut options = String::from("-o BatchMode=yes");
    if let Some(identity) = &device.identity_file {
        options.push_str(" -i ");
        options.push_str(&identity.to_string_lossy());
    }
    environment.insert("DOCKER_SSH_OPTS".into(), options.into());
    environment
}

fn output_reason(output: &CheckOutput) -> String {
    let text = if output.stderr.trim().is_empty() {
        output.stdout.trim()
    } else {
        output.stderr.trim()
    };
    let text = text.chars().take(2_000).collect::<String>();
    if text.is_empty() {
        output.exit_code.map_or_else(
            || "terminated by signal without an error message".into(),
            |code| format!("exited with status {code} without an error message"),
        )
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, path::PathBuf};

    type CheckCall = (OsString, Vec<OsString>, BTreeMap<OsString, OsString>);

    #[derive(Default)]
    struct FakeExecutor {
        outputs: RefCell<Vec<io::Result<CheckOutput>>>,
        calls: RefCell<Vec<CheckCall>>,
    }

    impl DeviceCheckExecutor for FakeExecutor {
        fn run(
            &self,
            program: &OsStr,
            arguments: &[OsString],
            environment: &BTreeMap<OsString, OsString>,
        ) -> io::Result<CheckOutput> {
            self.calls.borrow_mut().push((
                program.to_owned(),
                arguments.to_vec(),
                environment.clone(),
            ));
            self.outputs.borrow_mut().remove(0)
        }
    }

    fn output(success: bool, stdout: &str, stderr: &str) -> io::Result<CheckOutput> {
        Ok(CheckOutput {
            success,
            exit_code: Some(if success { 0 } else { 1 }),
            stdout: stdout.into(),
            stderr: stderr.into(),
        })
    }

    fn device() -> RegisteredDevice {
        RegisteredDevice {
            name: "builder".into(),
            host: "192.0.2.10".into(),
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
    fn checks_ssh_then_docker_with_native_ssh_transport() {
        let executor = FakeExecutor {
            outputs: RefCell::new(vec![output(true, "", ""), output(true, "28.5.1\n", "")]),
            ..Default::default()
        };
        assert_eq!(
            check_device_eligibility_with(&executor, &device()),
            (
                DeviceCheckStatus::Eligible,
                "eligible for remote container execution (docker 28.5.1)".into()
            )
        );
        let calls = executor.calls.borrow();
        assert_eq!(calls[0].0, "ssh");
        assert!(calls[0].1.contains(&OsString::from("BatchMode=yes")));
        assert_eq!(calls[1].0, "docker");
        assert_eq!(calls[1].1[2], "{{.Server.Version}}");
        assert_eq!(
            calls[1].2[OsStr::new("DOCKER_HOST")],
            "ssh://dev@192.0.2.10:2222"
        );
        assert!(
            calls[1].2[OsStr::new("DOCKER_SSH_OPTS")]
                .to_string_lossy()
                .contains("keys/id_ed25519")
        );
    }

    #[test]
    fn reports_ssh_and_docker_failures_concretely() {
        let ssh = FakeExecutor {
            outputs: RefCell::new(vec![output(false, "", "Permission denied (publickey).")]),
            ..Default::default()
        };
        let result = check_device_eligibility_with(&ssh, &device());
        assert_eq!(result.0, DeviceCheckStatus::AuthFailed);
        assert!(result.1.contains("SSH failed: Permission denied"));

        let docker = FakeExecutor {
            outputs: RefCell::new(vec![
                output(true, "", ""),
                output(false, "", "docker: command not found"),
            ]),
            ..Default::default()
        };
        let result = check_device_eligibility_with(&docker, &device());
        assert_eq!(result.0, DeviceCheckStatus::Ineligible);
        assert_eq!(result.1, "no docker over SSH: docker: command not found");
    }
}
