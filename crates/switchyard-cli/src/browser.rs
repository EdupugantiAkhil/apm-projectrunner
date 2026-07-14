use std::{
    env, fmt, fs, io,
    net::{SocketAddr, TcpStream},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

use serde::Deserialize;

const API_VERSION: &str = "switchyard.dev/managed-profile/v1alpha1";
const AUTH_OWNER_MARKER: &[u8] = b"switchyard-managed-profile-auth-v1\n";
const MAX_PROXY_CREDENTIAL_BYTES: u64 = 256;
const BROWSER_CANDIDATES: &[&str] = &["chromium", "chromium-browser"];

#[derive(Debug)]
pub enum BrowserError {
    Io(io::Error),
    InvalidMetadata(String),
    InvalidCredential(String),
    UnsupportedBrowser,
    ProxyUnavailable(SocketAddr),
}

impl fmt::Display for BrowserError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(formatter),
            Self::InvalidMetadata(message) => {
                write!(formatter, "managed profile metadata is invalid: {message}")
            }
            Self::InvalidCredential(message) => {
                write!(formatter, "managed proxy credential is invalid: {message}")
            }
            Self::UnsupportedBrowser => write!(
                formatter,
                "no supported Chromium build found; install Chromium or set SWITCHYARD_CHROMIUM to Chromium or Chrome for Testing (branded Chrome and Edge do not support the required extension launch flag)"
            ),
            Self::ProxyUnavailable(address) => write!(
                formatter,
                "managed proxy {address} is not running; start the deployment host gateway before opening the profile"
            ),
        }
    }
}

impl std::error::Error for BrowserError {}

impl From<io::Error> for BrowserError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedProfile {
    pub api_version: String,
    pub deployment: String,
    pub ui: String,
    pub route: String,
    pub proxy_address: SocketAddr,
    pub start_url: String,
}

pub fn load_managed_profile(
    workspace_root: &Path,
    artifact_dir: &Path,
    expected_deployment: &str,
    ui: &str,
) -> Result<ManagedProfile, BrowserError> {
    validate_identifier(ui)?;
    let canonical_workspace = fs::canonicalize(workspace_root)?;
    let generated_base = fs::canonicalize(workspace_root.join(".switchyard/generated"))?;
    require_contained(
        &canonical_workspace,
        &generated_base,
        "generated directory escapes the workspace",
    )?;
    let generated_root = fs::canonicalize(workspace_root.join(artifact_dir))?;
    require_contained(
        &generated_base,
        &generated_root,
        "deployment artifacts are outside the generated directory",
    )?;
    let profiles_root = fs::canonicalize(generated_root.join("managed-profiles"))?;
    require_contained(
        &generated_root,
        &profiles_root,
        "managed profile directory escapes the deployment artifacts",
    )?;
    let metadata_path = profiles_root.join(format!("{ui}.json"));
    let file_type = fs::symlink_metadata(&metadata_path)?.file_type();
    if !file_type.is_file() || file_type.is_symlink() {
        return Err(BrowserError::InvalidMetadata(
            "profile metadata must be a regular generated file".into(),
        ));
    }
    let canonical_metadata = fs::canonicalize(&metadata_path)?;
    require_contained(
        &profiles_root,
        &canonical_metadata,
        "profile metadata escapes the managed profile directory",
    )?;
    let metadata: ManagedProfile = serde_json::from_slice(&fs::read(&canonical_metadata)?)
        .map_err(|error| BrowserError::InvalidMetadata(error.to_string()))?;
    validate_profile(metadata, expected_deployment, ui)
}

fn require_contained(root: &Path, candidate: &Path, message: &str) -> Result<(), BrowserError> {
    if candidate.starts_with(root) {
        Ok(())
    } else {
        Err(BrowserError::InvalidMetadata(message.into()))
    }
}

fn validate_profile(
    metadata: ManagedProfile,
    expected_deployment: &str,
    ui: &str,
) -> Result<ManagedProfile, BrowserError> {
    validate_identifier(&metadata.deployment)?;
    validate_identifier(&metadata.ui)?;
    validate_identifier(&metadata.route)?;
    if metadata.api_version != API_VERSION {
        return Err(BrowserError::InvalidMetadata(format!(
            "expected apiVersion {API_VERSION}"
        )));
    }
    if metadata.deployment != expected_deployment || metadata.ui != ui {
        return Err(BrowserError::InvalidMetadata(
            "deployment or UI ownership does not match the requested profile".into(),
        ));
    }
    if metadata.route.is_empty() {
        return Err(BrowserError::InvalidMetadata(
            "route must not be empty".into(),
        ));
    }
    if !metadata.proxy_address.ip().is_loopback() || metadata.proxy_address.port() == 0 {
        return Err(BrowserError::InvalidMetadata(
            "proxyAddress must be a nonzero loopback listener".into(),
        ));
    }
    validate_start_url(&metadata.start_url)?;
    Ok(metadata)
}

fn validate_identifier(value: &str) -> Result<(), BrowserError> {
    let valid = !value.is_empty()
        && value.len() <= 63
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric);
    if valid {
        Ok(())
    } else {
        Err(BrowserError::InvalidMetadata(format!(
            "UI `{value}` is not a safe deployment identifier"
        )))
    }
}

fn validate_start_url(value: &str) -> Result<(), BrowserError> {
    if switchyard_planner::is_local_http_url(value) {
        Ok(())
    } else {
        Err(BrowserError::InvalidMetadata(
            "startUrl must be a well-formed local HTTP URL without credentials, whitespace, or a fragment"
                .into(),
        ))
    }
}

pub fn open_managed_profile(
    workspace_root: &Path,
    profile: &ManagedProfile,
) -> Result<PathBuf, BrowserError> {
    validate_identifier(&profile.deployment)?;
    validate_identifier(&profile.ui)?;
    if TcpStream::connect_timeout(&profile.proxy_address, Duration::from_millis(500)).is_err() {
        return Err(BrowserError::ProxyUnavailable(profile.proxy_address));
    }
    let executable = find_browser()?;
    let profile_dir = ensure_profile_directory(workspace_root, profile)?;
    let credential = load_proxy_credential(workspace_root, profile)?;
    let authentication_extension = materialize_auth_extension(&profile_dir, &credential)?;
    let arguments = browser_arguments(profile, &profile_dir, &authentication_extension);
    Command::new(&executable)
        .args(&arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(profile_dir)
}

fn ensure_profile_directory(
    workspace_root: &Path,
    profile: &ManagedProfile,
) -> Result<PathBuf, BrowserError> {
    let canonical_workspace = fs::canonicalize(workspace_root)?;
    let switchyard = workspace_root.join(".switchyard");
    let metadata = fs::symlink_metadata(&switchyard)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(BrowserError::InvalidCredential(
            ".switchyard must be a real directory".into(),
        ));
    }
    let mut current = fs::canonicalize(switchyard)?;
    require_contained(
        &canonical_workspace,
        &current,
        "profile root escapes the workspace",
    )?;
    for component in ["profiles", &profile.deployment, &profile.ui] {
        let next = current.join(component);
        match fs::symlink_metadata(&next) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(BrowserError::InvalidCredential(format!(
                    "profile path component `{component}` is not a real directory"
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                match fs::create_dir(&next) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error.into()),
                }
                let metadata = fs::symlink_metadata(&next)?;
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(BrowserError::InvalidCredential(format!(
                        "profile path component `{component}` is not a real directory"
                    )));
                }
            }
            Err(error) => return Err(error.into()),
        }
        fs::set_permissions(&next, fs::Permissions::from_mode(0o700))?;
        let canonical = fs::canonicalize(&next)?;
        require_contained(&current, &canonical, "profile directory escapes its parent")?;
        current = canonical;
    }
    Ok(current)
}

fn load_proxy_credential(
    workspace_root: &Path,
    profile: &ManagedProfile,
) -> Result<String, BrowserError> {
    let canonical_workspace = fs::canonicalize(workspace_root)?;
    let runtime_root = fs::canonicalize(workspace_root.join(".switchyard/run"))?;
    require_contained(
        &canonical_workspace,
        &runtime_root,
        "runtime directory escapes the workspace",
    )?;
    let deployment_root = fs::canonicalize(runtime_root.join(&profile.deployment))?;
    require_contained(
        &runtime_root,
        &deployment_root,
        "deployment runtime directory escapes Switchyard runtime state",
    )?;
    let profiles_root = fs::canonicalize(deployment_root.join("managed-profiles"))?;
    require_contained(
        &deployment_root,
        &profiles_root,
        "managed credential directory escapes deployment runtime state",
    )?;
    let credential_path = profiles_root.join(format!("{}.credential", profile.ui));
    let metadata = fs::symlink_metadata(&credential_path)?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.permissions().mode() & 0o777 != 0o600
    {
        return Err(BrowserError::InvalidCredential(
            "credential must be a regular owner-only file".into(),
        ));
    }
    if metadata.len() > MAX_PROXY_CREDENTIAL_BYTES {
        return Err(BrowserError::InvalidCredential(format!(
            "credential exceeds {MAX_PROXY_CREDENTIAL_BYTES} bytes"
        )));
    }
    let canonical_credential = fs::canonicalize(&credential_path)?;
    require_contained(
        &profiles_root,
        &canonical_credential,
        "credential escapes the managed credential directory",
    )?;
    let credential = fs::read_to_string(canonical_credential)?;
    let credential = credential.trim();
    if credential.is_empty() || credential.chars().any(char::is_control) {
        return Err(BrowserError::InvalidCredential(
            "credential is empty or malformed".into(),
        ));
    }
    Ok(credential.to_owned())
}

fn materialize_auth_extension(
    profile_dir: &Path,
    credential: &str,
) -> Result<PathBuf, BrowserError> {
    let profile_dir = fs::canonicalize(profile_dir)?;
    let requested = profile_dir.join(".switchyard-proxy-auth");
    let created = match fs::symlink_metadata(&requested) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(BrowserError::InvalidCredential(
                "authentication helper path is not a real directory".into(),
            ));
        }
        Ok(_) => false,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(&requested)?;
            true
        }
        Err(error) => return Err(error.into()),
    };
    fs::set_permissions(&requested, fs::Permissions::from_mode(0o700))?;
    let directory = fs::canonicalize(&requested)?;
    require_contained(
        &profile_dir,
        &directory,
        "authentication helper escapes the managed profile",
    )?;
    let marker = directory.join(".switchyard-owned");
    if created {
        write_new_private(&marker, AUTH_OWNER_MARKER)?;
    } else {
        require_owned_file(&marker, Some(AUTH_OWNER_MARKER))?;
    }
    atomic_replace_owned(
        &directory,
        "manifest.json",
        br#"{"manifest_version":3,"name":"Switchyard managed proxy authentication","version":"0.1.0","permissions":["webRequest","webRequestAuthProvider"],"host_permissions":["<all_urls>"],"background":{"service_worker":"service-worker.js"}}"#,
    )?;
    let encoded = serde_json::to_string(credential)
        .map_err(|error| BrowserError::InvalidCredential(error.to_string()))?;
    let worker = format!(
        "const PASSWORD={encoded};chrome.webRequest.onAuthRequired.addListener((details,callback)=>{{if(details.isProxy){{callback({{authCredentials:{{username:'switchyard',password:PASSWORD}}}});}}else{{callback();}}}},{{urls:['<all_urls>']}},['asyncBlocking']);\n"
    );
    atomic_replace_owned(&directory, "service-worker.js", worker.as_bytes())?;
    Ok(directory)
}

fn write_new_private(path: &Path, contents: &[u8]) -> Result<(), BrowserError> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true).mode(0o600);
    let mut file = options.open(path)?;
    use std::io::Write;
    file.write_all(contents)?;
    file.sync_all()?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

fn require_owned_file(path: &Path, expected: Option<&[u8]>) -> Result<(), BrowserError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.permissions().mode() & 0o777 != 0o600
    {
        return Err(BrowserError::InvalidCredential(format!(
            "{} is not an owned regular file",
            path.display()
        )));
    }
    if expected.is_some_and(|contents| fs::read(path).ok().as_deref() != Some(contents)) {
        return Err(BrowserError::InvalidCredential(format!(
            "{} has an invalid ownership marker",
            path.display()
        )));
    }
    Ok(())
}

fn atomic_replace_owned(directory: &Path, name: &str, contents: &[u8]) -> Result<(), BrowserError> {
    let destination = directory.join(name);
    match fs::symlink_metadata(&destination) {
        Ok(_) => require_owned_file(&destination, None)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let temporary = directory.join(format!(
        ".{name}.tmp-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    write_new_private(&temporary, contents)?;
    if let Err(error) = fs::rename(&temporary, &destination) {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    fs::File::open(directory)?.sync_all()?;
    Ok(())
}

fn browser_arguments(
    profile: &ManagedProfile,
    profile_dir: &Path,
    authentication_extension: &Path,
) -> Vec<String> {
    vec![
        format!("--user-data-dir={}", profile_dir.display()),
        format!("--proxy-server=http://{}", profile.proxy_address),
        "--proxy-bypass-list=<-loopback>".into(),
        format!(
            "--disable-extensions-except={}",
            authentication_extension.display()
        ),
        format!("--load-extension={}", authentication_extension.display()),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
        profile.start_url.clone(),
    ]
}

fn find_browser() -> Result<PathBuf, BrowserError> {
    if let Some(configured) = env::var_os("SWITCHYARD_CHROMIUM") {
        let executable = find_executable(&configured).ok_or(BrowserError::UnsupportedBrowser)?;
        if !supported_browser_executable(&executable) {
            return Err(BrowserError::UnsupportedBrowser);
        }
        return Ok(executable);
    }
    BROWSER_CANDIDATES
        .iter()
        .filter_map(|candidate| find_executable(candidate.as_ref()))
        .find(|candidate| supported_browser_executable(candidate))
        .ok_or(BrowserError::UnsupportedBrowser)
}

fn supported_browser_executable(executable: &Path) -> bool {
    let Ok(output) = Command::new(executable).arg("--version").output() else {
        return false;
    };
    let version = format!(
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output.status.success() && supported_browser_version(&version)
}

fn supported_browser_version(version: &str) -> bool {
    let normalized = version.to_ascii_lowercase();
    normalized.contains("chromium") || normalized.contains("chrome for testing")
}

fn find_executable(program: &std::ffi::OsStr) -> Option<PathBuf> {
    let path = Path::new(program);
    if path.components().count() > 1 {
        return executable(path).then(|| path.to_owned());
    }
    env::split_paths(&env::var_os("PATH")?).find_map(|directory| {
        let candidate = directory.join(path);
        executable(&candidate).then_some(candidate)
    })
}

fn executable(path: &Path) -> bool {
    fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn profile() -> ManagedProfile {
        ManagedProfile {
            api_version: API_VERSION.into(),
            deployment: "comparison".into(),
            ui: "ui-1".into(),
            route: "ui-1".into(),
            proxy_address: "127.0.0.1:24101".parse().unwrap(),
            start_url: "http://ui-1.comparison.localhost/".into(),
        }
    }

    #[test]
    fn chromium_arguments_disable_the_implicit_loopback_bypass() {
        let arguments = browser_arguments(
            &profile(),
            Path::new("/tmp/profile"),
            Path::new("/tmp/profile/auth"),
        );
        assert!(arguments.contains(&"--proxy-server=http://127.0.0.1:24101".into()));
        assert!(arguments.contains(&"--proxy-bypass-list=<-loopback>".into()));
        assert!(arguments.contains(&"--user-data-dir=/tmp/profile".into()));
        assert!(arguments.contains(&"--load-extension=/tmp/profile/auth".into()));
    }

    #[test]
    fn rejects_non_loopback_proxy_metadata() {
        let mut profile = profile();
        profile.proxy_address = "192.0.2.10:24101".parse().unwrap();
        assert!(validate_profile(profile, "comparison", "ui-1").is_err());
    }

    #[test]
    fn rejects_metadata_owned_by_another_deployment() {
        let profile = profile();
        assert!(validate_profile(profile, "other", "ui-1").is_err());
    }

    #[test]
    fn validates_only_local_http_start_urls() {
        for valid in [
            "http://localhost:3000/",
            "http://ui.comparison.localhost/",
            "http://127.0.0.1:8080/",
            "http://[::1]:8080/",
        ] {
            assert!(validate_start_url(valid).is_ok(), "{valid}");
        }
        for invalid in [
            "https://ui.localhost/",
            "http://example.com/",
            "http://192.168.1.2/",
            "http://user:pass@localhost/",
            "http://localhost:99999/",
            "http://localhost/#fragment",
            "http://local host/",
            "--incognito",
        ] {
            assert!(validate_start_url(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn only_accepts_safe_ui_names_for_metadata_paths() {
        assert!(validate_identifier("ui-1").is_ok());
        assert!(validate_identifier("../ui-1").is_err());
        assert!(validate_identifier("UI One").is_err());
    }

    #[test]
    fn loopback_detection_accepts_ipv4_and_ipv6() {
        assert!(
            "127.0.0.1"
                .parse::<std::net::IpAddr>()
                .unwrap()
                .is_loopback()
        );
        assert!("::1".parse::<std::net::IpAddr>().unwrap().is_loopback());
    }

    #[test]
    fn rejects_a_symlinked_generated_ancestor_that_escapes_workspace() {
        let root = std::env::temp_dir().join(format!(
            "switchyard-profile-path-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let workspace = root.join("workspace");
        let outside = root.join("outside");
        fs::create_dir_all(workspace.join(".switchyard")).unwrap();
        fs::create_dir_all(outside.join("comparison/managed-profiles")).unwrap();
        fs::write(
            outside.join("comparison/managed-profiles/ui-1.json"),
            r#"{"apiVersion":"switchyard.dev/managed-profile/v1alpha1","deployment":"comparison","ui":"ui-1","route":"ui-1","proxyAddress":"127.0.0.1:24001","startUrl":"http://ui-1.localhost/"}"#,
        )
        .unwrap();
        symlink(&outside, workspace.join(".switchyard/generated")).unwrap();

        let result = load_managed_profile(
            &workspace,
            Path::new(".switchyard/generated/comparison"),
            "comparison",
            "ui-1",
        );
        assert!(result.is_err());
        assert!(
            outside
                .join("comparison/managed-profiles/ui-1.json")
                .is_file()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn accepts_only_chromium_or_chrome_for_testing_version_output() {
        assert!(supported_browser_version("Chromium 140.0.7339.0"));
        assert!(supported_browser_version(
            "Google Chrome for Testing 140.0.7339.0"
        ));
        assert!(!supported_browser_version("Google Chrome 140.0.7339.0"));
        assert!(!supported_browser_version("Microsoft Edge 140.0.7339.0"));
    }

    #[test]
    fn proxy_auth_helper_is_private_to_the_managed_profile() {
        let root = std::env::temp_dir().join(format!(
            "switchyard-profile-auth-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let extension = materialize_auth_extension(&root, "private-token").unwrap();
        assert_eq!(
            fs::metadata(&extension).unwrap().permissions().mode() & 0o777,
            0o700
        );
        for file in ["manifest.json", "service-worker.js"] {
            assert_eq!(
                fs::metadata(extension.join(file))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        assert!(
            fs::read_to_string(extension.join("service-worker.js"))
                .unwrap()
                .contains("private-token")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn auth_helper_refuses_a_symlink_without_clobbering_its_target() {
        let root = std::env::temp_dir().join(format!(
            "switchyard-profile-auth-link-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let profile_dir = root.join("profile");
        let target = root.join("outside.txt");
        fs::create_dir_all(&profile_dir).unwrap();
        let extension = materialize_auth_extension(&profile_dir, "first-token").unwrap();
        fs::write(&target, "keep-me").unwrap();
        fs::remove_file(extension.join("service-worker.js")).unwrap();
        symlink(&target, extension.join("service-worker.js")).unwrap();

        assert!(materialize_auth_extension(&profile_dir, "second-token").is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "keep-me");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_an_oversized_proxy_credential_before_reading_it() {
        let root = std::env::temp_dir().join(format!(
            "switchyard-profile-credential-size-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let credential_dir = root.join(".switchyard/run/comparison/managed-profiles");
        fs::create_dir_all(&credential_dir).unwrap();
        let credential = credential_dir.join("ui-1.credential");
        fs::write(&credential, vec![b'x'; 257]).unwrap();
        fs::set_permissions(&credential, fs::Permissions::from_mode(0o600)).unwrap();

        let error = load_proxy_credential(&root, &profile()).unwrap_err();
        assert!(error.to_string().contains("exceeds 256 bytes"));
        fs::remove_dir_all(root).unwrap();
    }
}
