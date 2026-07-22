//! Process-scoped Docker-over-SSH configuration.
//!
//! Docker's SSH connection helper invokes a program named `ssh`, but does not honor
//! `DOCKER_SSH_OPTS`. When an explicit identity is selected, this crate places a
//! private launcher first on `PATH` so the identity is passed to OpenSSH as a distinct
//! argument. The launcher's lifetime is tied to [`DockerSshTransport`].

use std::{
    collections::BTreeMap,
    env,
    ffi::OsString,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
};

const REAL_SSH_ENV: &str = "SWITCHYARD_DOCKER_REAL_SSH";
const IDENTITY_ENV: &str = "SWITCHYARD_DOCKER_SSH_IDENTITY";
const LAUNCHER: &[u8] = b"#!/bin/sh\nexec \"$SWITCHYARD_DOCKER_REAL_SSH\" -o BatchMode=yes -o IdentitiesOnly=yes -i \"$SWITCHYARD_DOCKER_SSH_IDENTITY\" \"$@\"\n";

/// Environment and owned temporary state for one Docker SSH operation.
#[derive(Debug)]
pub struct DockerSshTransport {
    environment: BTreeMap<OsString, OsString>,
    _launcher_directory: Option<tempfile::TempDir>,
}

impl DockerSshTransport {
    pub fn new(
        user: &str,
        host: &str,
        port: u16,
        identity_file: Option<&Path>,
    ) -> io::Result<Self> {
        Self::new_with_resolver(user, host, port, identity_file, resolve_ssh)
    }

    fn new_with_resolver<F>(
        user: &str,
        host: &str,
        port: u16,
        identity_file: Option<&Path>,
        resolve: F,
    ) -> io::Result<Self>
    where
        F: FnOnce() -> io::Result<PathBuf>,
    {
        let mut environment = BTreeMap::from([(
            OsString::from("DOCKER_HOST"),
            format!("ssh://{user}@{host}:{port}").into(),
        )]);
        let Some(identity_file) = identity_file else {
            return Ok(Self {
                environment,
                _launcher_directory: None,
            });
        };

        let real_ssh = resolve()?;
        let directory = tempfile::Builder::new()
            .prefix("switchyard-docker-ssh-")
            .tempdir()?;
        set_private_directory(directory.path())?;
        let launcher = directory.path().join("ssh");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&launcher)?;
        file.write_all(LAUNCHER)?;
        file.sync_all()?;
        set_executable(&launcher)?;

        let inherited_path = env::var_os("PATH").unwrap_or_else(|| OsString::from("/usr/bin:/bin"));
        let path = env::join_paths(
            std::iter::once(directory.path().to_path_buf())
                .chain(env::split_paths(&inherited_path)),
        )
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        environment.insert("PATH".into(), path);
        environment.insert(REAL_SSH_ENV.into(), real_ssh.into_os_string());
        environment.insert(IDENTITY_ENV.into(), identity_file.as_os_str().to_owned());

        Ok(Self {
            environment,
            _launcher_directory: Some(directory),
        })
    }

    pub fn environment(&self) -> &BTreeMap<OsString, OsString> {
        &self.environment
    }

    pub fn apply(&self, command: &mut Command) {
        command.envs(&self.environment);
    }
}

fn resolve_ssh() -> io::Result<PathBuf> {
    let path = env::var_os("PATH").unwrap_or_else(|| OsString::from("/usr/bin:/bin"));
    env::split_paths(&path)
        .map(|directory| directory.join("ssh"))
        .find(|candidate| candidate.is_file() && is_executable(candidate))
        .map(fs::canonicalize)
        .transpose()?
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "system `ssh` binary is unavailable",
            )
        })
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(unix)]
fn set_private_directory(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn set_private_directory(_path: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "explicit Docker SSH identities require a Unix host",
    ))
}

#[cfg(unix)]
fn set_executable(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "explicit Docker SSH identities require a Unix host",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::process::Stdio;

    #[test]
    fn no_identity_preserves_normal_ssh_resolution() {
        let transport =
            DockerSshTransport::new_with_resolver("dev", "host.test", 2222, None, || {
                panic!("ssh must not be resolved without an identity")
            })
            .unwrap();
        assert_eq!(
            transport.environment()[OsStr::new("DOCKER_HOST")],
            "ssh://dev@host.test:2222"
        );
        assert!(!transport.environment().contains_key(OsStr::new("PATH")));
    }

    #[cfg(unix)]
    #[test]
    fn launcher_passes_identity_and_options_as_distinct_arguments() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let fake_ssh = directory.path().join("real ssh");
        fs::write(&fake_ssh, b"#!/bin/sh\nprintf '<%s>\\n' \"$@\"\n").unwrap();
        fs::set_permissions(&fake_ssh, fs::Permissions::from_mode(0o700)).unwrap();
        let identity = directory.path().join("key with spaces");
        let transport = DockerSshTransport::new_with_resolver(
            "dev",
            "host.test",
            2222,
            Some(&identity),
            || Ok(fake_ssh),
        )
        .unwrap();
        let launcher_directory = env::split_paths(&transport.environment()[OsStr::new("PATH")])
            .next()
            .unwrap();
        assert_eq!(
            fs::metadata(&launcher_directory)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        let launcher = launcher_directory.join("ssh");
        let mut command = Command::new(launcher);
        command
            .args([
                "-l",
                "dev",
                "-p",
                "2222",
                "host.test",
                "docker system dial-stdio",
            ])
            .stdout(Stdio::piped());
        transport.apply(&mut command);
        let output = command.output().unwrap();
        assert!(output.status.success());
        let arguments = String::from_utf8(output.stdout).unwrap();
        assert!(arguments.contains("<-o>\n<BatchMode=yes>"));
        assert!(arguments.contains("<-o>\n<IdentitiesOnly=yes>"));
        assert!(arguments.contains(&format!("<-i>\n<{}>", identity.display())));
        assert!(arguments.contains("<-l>\n<dev>\n<-p>\n<2222>\n<host.test>"));
        drop(transport);
        assert!(!launcher_directory.exists());
    }
}
