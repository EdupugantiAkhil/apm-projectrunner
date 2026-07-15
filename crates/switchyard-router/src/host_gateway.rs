//! Linux-first host listener preflight and managed local certificates.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs, io,
    net::{IpAddr, TcpListener},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use rcgen::{CertificateParams, DnType, KeyPair};
use router_config::{ListenerDestination, Protocol, RouterConfig};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

const CERTIFICATE_API_VERSION: &str = "switchyard.dev/certificate/v1alpha1";
const CERTIFICATE_LIFETIME: Duration = Duration::from_secs(90 * 24 * 60 * 60);
const RENEW_BEFORE: Duration = Duration::from_secs(30 * 24 * 60 * 60);

#[derive(Debug)]
pub enum HostGatewayError {
    InvalidConfiguration(String),
    PortConflict { address: String, source: io::Error },
    DomainConflict { domain: String },
    UnsafeUpstream { provider: String, host: String },
    Certificate { path: PathBuf, message: String },
    Io(io::Error),
}

impl fmt::Display for HostGatewayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfiguration(message) => {
                write!(formatter, "invalid host gateway: {message}")
            }
            Self::PortConflict { address, source } => {
                write!(
                    formatter,
                    "host listener {address} is unavailable: {source}"
                )
            }
            Self::DomainConflict { domain } => {
                write!(
                    formatter,
                    "custom domain `{domain}` is declared more than once"
                )
            }
            Self::UnsafeUpstream { provider, host } => write!(
                formatter,
                "host provider `{provider}` must use a loopback upstream, not `{host}`"
            ),
            Self::Certificate { path, message } => {
                write!(formatter, "certificate {}: {message}", path.display())
            }
            Self::Io(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for HostGatewayError {}

impl From<io::Error> for HostGatewayError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CertificateReport {
    pub generated: Vec<PathBuf>,
    pub renewed: Vec<PathBuf>,
    pub external: Vec<PathBuf>,
}

/// Checks all host claims without writing files or starting a partial data plane.
pub fn preflight(config: &RouterConfig) -> Result<(), HostGatewayError> {
    config.validate().map_err(|errors| {
        HostGatewayError::InvalidConfiguration(
            errors
                .into_iter()
                .map(|error| error.to_string())
                .collect::<Vec<_>>()
                .join("; "),
        )
    })?;

    let mut domains = BTreeMap::new();
    for listener in &config.spec.listeners {
        if !listener.bind.host.is_loopback() {
            return Err(HostGatewayError::InvalidConfiguration(format!(
                "listener {}:{} is not loopback-bound",
                listener.bind.host, listener.bind.port
            )));
        }
        let mut listener_hosts = BTreeMap::new();
        for destination in &listener.destinations {
            let (slot, host) = match destination {
                ListenerDestination::CustomDomain { slot, domain } => {
                    (slot, Some(normalize_domain(domain)))
                }
                ListenerDestination::LegacyLocalhost { slot, host } => {
                    (slot, Some(normalize_domain(host)))
                }
                ListenerDestination::Loopback { slot } => {
                    if listener.destinations.len() != 1 {
                        return Err(HostGatewayError::InvalidConfiguration(format!(
                            "listener {}:{} must not mix a loopback destination with other slots",
                            listener.bind.host, listener.bind.port
                        )));
                    }
                    (slot, None)
                }
                ListenerDestination::ProxyTarget {
                    slot, host, port, ..
                } => (slot, Some(format!("{}:{port}", normalize_domain(host)))),
            };
            if let Some(host) = host {
                if listener_hosts
                    .insert(host.clone(), slot.clone())
                    .is_some_and(|first| first != *slot)
                {
                    return Err(HostGatewayError::InvalidConfiguration(format!(
                        "listener {}:{} maps host `{host}` to multiple route slots",
                        listener.bind.host, listener.bind.port
                    )));
                }
            }
            if listener.proxy_identity.is_none() {
                if let ListenerDestination::CustomDomain { slot, domain } = destination {
                    let normalized = normalize_domain(domain);
                    if domains
                        .insert(normalized.clone(), slot.clone())
                        .is_some_and(|first| first != *slot)
                    {
                        return Err(HostGatewayError::DomainConflict { domain: normalized });
                    }
                }
            }
        }
    }

    for provider in &config.spec.providers {
        if !is_loopback_host(&provider.endpoint.host) {
            return Err(HostGatewayError::UnsafeUpstream {
                provider: provider.id.to_string(),
                host: provider.endpoint.host.clone(),
            });
        }
    }

    // Hold all sockets until every claim has succeeded. This avoids discovering the
    // second conflict after a first listener has already started.
    let mut reservations = Vec::new();
    for listener in &config.spec.listeners {
        let address = format!("{}:{}", listener.bind.host, listener.bind.port);
        let socket =
            TcpListener::bind((listener.bind.host, listener.bind.port)).map_err(|source| {
                HostGatewayError::PortConflict {
                    address: address.clone(),
                    source,
                }
            })?;
        reservations.push(socket);
    }
    drop(reservations);
    Ok(())
}

/// Generates missing managed identities and renews those within 30 days of expiry.
/// Existing unmarked certificate/key pairs are treated as user-managed and untouched.
pub fn ensure_certificates(config: &RouterConfig) -> Result<CertificateReport, HostGatewayError> {
    let mut identities = BTreeMap::<(PathBuf, PathBuf), BTreeSet<String>>::new();
    for listener in &config.spec.listeners {
        let Some(tls) = &listener.tls else { continue };
        if listener.protocol != Protocol::Https {
            return Err(certificate_error(
                &tls.certificate,
                "TLS identity is only valid for an HTTPS listener",
            ));
        }
        let domains = identities
            .entry((tls.certificate.clone(), tls.private_key.clone()))
            .or_default();
        for destination in &listener.destinations {
            match destination {
                ListenerDestination::CustomDomain { domain, .. } => {
                    domains.insert(normalize_domain(domain));
                }
                ListenerDestination::LegacyLocalhost { host, .. } => {
                    domains.insert(normalize_domain(host));
                }
                ListenerDestination::Loopback { .. } => {
                    domains.insert("localhost".into());
                }
                ListenerDestination::ProxyTarget { host, .. } => {
                    domains.insert(normalize_domain(host));
                }
            }
        }
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut report = CertificateReport::default();
    for ((certificate, private_key), domains) in identities {
        if domains.is_empty() {
            return Err(certificate_error(
                &certificate,
                "HTTPS identity has no DNS names",
            ));
        }
        reject_symlink_and_parents(&certificate)?;
        reject_symlink_and_parents(&private_key)?;
        let marker_path = marker_path(&certificate);
        reject_symlink_and_parents(&marker_path)?;
        let certificate_exists = regular_file_metadata(&certificate)?.is_some();
        let key_exists = regular_file_metadata(&private_key)?.is_some();
        if certificate_exists != key_exists {
            return Err(certificate_error(
                &certificate,
                "certificate and private key must either both exist or both be absent",
            ));
        }

        let marker = read_marker(&marker_path)?;
        if certificate_exists && marker.is_none() {
            report.external.push(certificate);
            continue;
        }
        if let Some(marker) = &marker {
            validate_certificate_marker(config, &certificate, &private_key, marker)?;
        }
        let expected_domains = domains.into_iter().collect::<Vec<_>>();
        let renew = marker.as_ref().is_some_and(|marker| {
            marker.not_after_unix <= now.saturating_add(RENEW_BEFORE.as_secs())
                || marker.domains != expected_domains
        });
        if !certificate_exists || renew {
            generate_identity(
                &certificate,
                &private_key,
                &marker_path,
                config.metadata.deployment.as_str(),
                &expected_domains,
                now,
            )?;
            if renew {
                report.renewed.push(certificate);
            } else {
                report.generated.push(certificate);
            }
        }
    }
    Ok(report)
}

/// Creates opaque per-listener proxy credentials with owner-only permissions.
pub fn ensure_proxy_credentials(config: &RouterConfig) -> Result<Vec<PathBuf>, HostGatewayError> {
    let mut generated = Vec::new();
    let mut seen = BTreeSet::new();
    for authentication in config
        .spec
        .listeners
        .iter()
        .filter_map(|listener| listener.proxy_authentication.as_ref())
    {
        let path = &authentication.credential_file;
        if !seen.insert(path.clone()) {
            return Err(certificate_error(
                path,
                "proxy credentials must not be shared between listeners",
            ));
        }
        reject_symlink_and_parents(path)?;
        let marker_path = credential_marker_path(path);
        reject_symlink_and_parents(&marker_path)?;
        if regular_file_metadata(path)?.is_some() {
            validate_proxy_credential(path)?;
            if let Some(marker) = read_credential_marker(&marker_path)? {
                validate_credential_marker(config, path, &marker)?;
            }
            continue;
        }
        create_private_parent(path)?;
        let mut random = [0_u8; 32];
        io::Read::read_exact(&mut fs::File::open("/dev/urandom")?, &mut random)?;
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut token = String::with_capacity(random.len() * 2);
        for byte in random {
            token.push(char::from(HEX[usize::from(byte >> 4)]));
            token.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        write_atomic(path, token.as_bytes(), 0o600)?;
        let marker = CredentialMarker {
            api_version: CERTIFICATE_API_VERSION.into(),
            deployment: config.metadata.deployment.to_string(),
            credential: fs::canonicalize(path)?,
            sha256: file_digest(path)?,
        };
        write_atomic(
            &marker_path,
            &serde_json::to_vec_pretty(&marker)
                .map_err(|error| certificate_error(&marker_path, error.to_string()))?,
            0o600,
        )?;
        generated.push(path.clone());
    }
    Ok(generated)
}

/// Removes only proxy credentials carrying a Switchyard ownership marker.
pub fn cleanup_proxy_credentials(config: &RouterConfig) -> Result<Vec<PathBuf>, HostGatewayError> {
    let mut removed = Vec::new();
    let mut seen = BTreeSet::new();
    for authentication in config
        .spec
        .listeners
        .iter()
        .filter_map(|listener| listener.proxy_authentication.as_ref())
    {
        let path = &authentication.credential_file;
        if !seen.insert(path.clone()) {
            continue;
        }
        reject_symlink_and_parents(path)?;
        let marker_path = credential_marker_path(path);
        let Some(marker) = read_credential_marker(&marker_path)? else {
            continue;
        };
        validate_credential_marker(config, path, &marker)?;
        remove_regular_file(path)?;
        remove_regular_file(&marker_path)?;
        removed.push(path.clone());
    }
    Ok(removed)
}

/// Removes only certificate/key pairs carrying a valid Switchyard ownership marker.
pub fn cleanup_certificates(config: &RouterConfig) -> Result<Vec<PathBuf>, HostGatewayError> {
    let mut removed = Vec::new();
    let mut seen = BTreeSet::new();
    for tls in config
        .spec
        .listeners
        .iter()
        .filter_map(|listener| listener.tls.as_ref())
    {
        if !seen.insert((tls.certificate.clone(), tls.private_key.clone())) {
            continue;
        }
        reject_symlink_and_parents(&tls.certificate)?;
        reject_symlink_and_parents(&tls.private_key)?;
        let marker_path = marker_path(&tls.certificate);
        let Some(marker) = read_marker(&marker_path)? else {
            continue;
        };
        validate_certificate_marker(config, &tls.certificate, &tls.private_key, &marker)?;
        remove_regular_file(&tls.private_key)?;
        remove_regular_file(&tls.certificate)?;
        remove_regular_file(&marker_path)?;
        removed.push(tls.certificate.clone());
    }
    Ok(removed)
}

/// Prints commands rather than mutating the operating-system trust store.
pub fn trust_guidance(config: &RouterConfig) -> String {
    let certificates = config
        .spec
        .listeners
        .iter()
        .filter_map(|listener| listener.tls.as_ref())
        .map(|tls| tls.certificate.display().to_string())
        .collect::<BTreeSet<_>>();
    if certificates.is_empty() {
        return "No HTTPS certificates are configured.".into();
    }
    format!(
        "Generated certificates are self-signed and are not installed automatically.\n\
         Linux (Debian/Ubuntu): copy each certificate to /usr/local/share/ca-certificates with a .crt suffix, then run `sudo update-ca-certificates`.\n\
         Firefox may use its own trust store; import the certificate under Authorities if system trust is disabled.\n\
         Remove the copied trust-store file and run `sudo update-ca-certificates --fresh` before `switchyard-router certificates cleanup`.\n\
         Configured certificates:\n  {}",
        certificates.into_iter().collect::<Vec<_>>().join("\n  ")
    )
}

fn generate_identity(
    certificate: &Path,
    private_key: &Path,
    marker_path: &Path,
    deployment: &str,
    domains: &[String],
    now_unix: u64,
) -> Result<(), HostGatewayError> {
    create_private_parent(certificate)?;
    create_private_parent(private_key)?;
    let now = OffsetDateTime::now_utc();
    let mut params = CertificateParams::new(domains.to_vec())
        .map_err(|error| certificate_error(certificate, error.to_string()))?;
    params.not_before = now - time::Duration::days(1);
    params.not_after = now + time::Duration::days(90);
    params
        .distinguished_name
        .push(DnType::CommonName, domains[0].clone());
    let key =
        KeyPair::generate().map_err(|error| certificate_error(private_key, error.to_string()))?;
    let cert = params
        .self_signed(&key)
        .map_err(|error| certificate_error(certificate, error.to_string()))?;
    write_atomic(private_key, key.serialize_pem().as_bytes(), 0o600)?;
    write_atomic(certificate, cert.pem().as_bytes(), 0o644)?;
    let marker = CertificateMarker {
        api_version: CERTIFICATE_API_VERSION.into(),
        deployment: deployment.into(),
        certificate: fs::canonicalize(certificate)?,
        private_key: fs::canonicalize(private_key)?,
        certificate_sha256: file_digest(certificate)?,
        private_key_sha256: file_digest(private_key)?,
        domains: domains.to_vec(),
        not_after_unix: now_unix.saturating_add(CERTIFICATE_LIFETIME.as_secs()),
    };
    write_atomic(
        marker_path,
        &serde_json::to_vec_pretty(&marker)
            .map_err(|error| certificate_error(marker_path, error.to_string()))?,
        0o600,
    )?;
    Ok(())
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CertificateMarker {
    api_version: String,
    deployment: String,
    certificate: PathBuf,
    private_key: PathBuf,
    certificate_sha256: String,
    private_key_sha256: String,
    domains: Vec<String>,
    not_after_unix: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CredentialMarker {
    api_version: String,
    deployment: String,
    credential: PathBuf,
    sha256: String,
}

fn read_marker(path: &Path) -> Result<Option<CertificateMarker>, HostGatewayError> {
    match read_marker_bytes(path)? {
        Some(bytes) => {
            let marker: CertificateMarker = serde_json::from_slice(&bytes).map_err(|error| {
                certificate_error(path, format!("invalid ownership marker: {error}"))
            })?;
            if marker.api_version != CERTIFICATE_API_VERSION {
                return Err(certificate_error(path, "unknown ownership marker version"));
            }
            Ok(Some(marker))
        }
        None => Ok(None),
    }
}

fn read_credential_marker(path: &Path) -> Result<Option<CredentialMarker>, HostGatewayError> {
    match read_marker_bytes(path)? {
        Some(bytes) => {
            let marker: CredentialMarker = serde_json::from_slice(&bytes).map_err(|error| {
                certificate_error(path, format!("invalid ownership marker: {error}"))
            })?;
            if marker.api_version != CERTIFICATE_API_VERSION {
                return Err(certificate_error(path, "unknown ownership marker version"));
            }
            Ok(Some(marker))
        }
        None => Ok(None),
    }
}

fn read_marker_bytes(path: &Path) -> Result<Option<Vec<u8>>, HostGatewayError> {
    reject_symlink_and_parents(path)?;
    let Some(metadata) = regular_file_metadata(path)? else {
        return Ok(None);
    };
    if metadata.permissions().mode() & 0o777 != 0o600 {
        return Err(certificate_error(
            path,
            "ownership marker must have mode 0600",
        ));
    }
    fs::read(path).map(Some).map_err(Into::into)
}

fn validate_certificate_marker(
    config: &RouterConfig,
    certificate: &Path,
    private_key: &Path,
    marker: &CertificateMarker,
) -> Result<(), HostGatewayError> {
    let certificate_path = fs::canonicalize(certificate)?;
    let private_key_path = fs::canonicalize(private_key)?;
    if marker.deployment != config.metadata.deployment.as_str()
        || marker.certificate != certificate_path
        || marker.private_key != private_key_path
        || marker.certificate_sha256 != file_digest(certificate)?
        || marker.private_key_sha256 != file_digest(private_key)?
    {
        return Err(certificate_error(
            &marker_path(certificate),
            "ownership marker does not match this deployment and certificate identity",
        ));
    }
    Ok(())
}

fn validate_credential_marker(
    config: &RouterConfig,
    credential: &Path,
    marker: &CredentialMarker,
) -> Result<(), HostGatewayError> {
    if marker.deployment != config.metadata.deployment.as_str()
        || marker.credential != fs::canonicalize(credential)?
        || marker.sha256 != file_digest(credential)?
    {
        return Err(certificate_error(
            &credential_marker_path(credential),
            "ownership marker does not match this deployment and credential identity",
        ));
    }
    Ok(())
}

fn marker_path(certificate: &Path) -> PathBuf {
    let mut value = certificate.as_os_str().to_owned();
    value.push(".switchyard.json");
    PathBuf::from(value)
}

fn credential_marker_path(credential: &Path) -> PathBuf {
    let mut value = credential.as_os_str().to_owned();
    value.push(".switchyard-owned");
    PathBuf::from(value)
}

fn normalize_domain(domain: &str) -> String {
    domain.trim_end_matches('.').to_ascii_lowercase()
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn reject_symlink_and_parents(path: &Path) -> Result<(), HostGatewayError> {
    ensure_parent_directories(path, false)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(certificate_error(path, "symbolic links are not allowed"))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn regular_file_metadata(path: &Path) -> Result<Option<fs::Metadata>, HostGatewayError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(Some(metadata)),
        Ok(_) => Err(certificate_error(
            path,
            "path must be a regular file and must not be a symbolic link",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn validate_proxy_credential(path: &Path) -> Result<(), HostGatewayError> {
    let metadata = regular_file_metadata(path)?
        .ok_or_else(|| certificate_error(path, "proxy credential file does not exist"))?;
    if metadata.len() == 0 || metadata.len() > 256 || metadata.permissions().mode() & 0o777 != 0o600
    {
        return Err(certificate_error(
            path,
            "proxy credential must be a regular mode-0600 file of 1 to 256 bytes",
        ));
    }
    let mut token = fs::read(path)?;
    if token.len() > 256 {
        token.fill(0);
        return Err(certificate_error(
            path,
            "proxy credential must be a regular mode-0600 file of 1 to 256 bytes",
        ));
    }
    if token.last() == Some(&b'\n') {
        token.pop();
        if token.last() == Some(&b'\r') {
            token.pop();
        }
    }
    let invalid = token.is_empty() || token.iter().any(|byte| matches!(byte, b'\r' | b'\n'));
    token.fill(0);
    if invalid {
        return Err(certificate_error(
            path,
            "proxy credential must contain one non-empty token",
        ));
    }
    Ok(())
}

fn file_digest(path: &Path) -> Result<String, HostGatewayError> {
    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let count = io::Read::read(&mut file, &mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn remove_regular_file(path: &Path) -> Result<(), HostGatewayError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => fs::remove_file(path).map_err(Into::into),
        Ok(_) => Err(certificate_error(
            path,
            "refusing to remove a non-regular file",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn create_private_parent(path: &Path) -> Result<(), HostGatewayError> {
    ensure_parent_directories(path, true)
}

fn ensure_parent_directories(path: &Path, create: bool) -> Result<(), HostGatewayError> {
    if path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(certificate_error(
            path,
            "managed paths must not contain parent-directory components",
        ));
    }
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    let Some(parent) = absolute.parent() else {
        return Ok(());
    };
    let mut current = PathBuf::new();
    for component in parent.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_dir() => {}
            Ok(_) => {
                return Err(certificate_error(
                    &current,
                    "parent path must be a real directory, not a symlink or other file type",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound && create => {
                fs::create_dir(&current)?;
                fs::set_permissions(&current, fs::Permissions::from_mode(0o700))?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn write_atomic(path: &Path, bytes: &[u8], mode: u32) -> Result<(), HostGatewayError> {
    create_private_parent(path)?;
    reject_symlink_and_parents(path)?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| certificate_error(path, "missing file name"))?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        name.to_string_lossy(),
        std::process::id()
    ));
    let result = (|| -> io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&temporary)?;
        io::Write::write_all(&mut file, bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(Into::into)
}

fn certificate_error(path: &Path, message: impl Into<String>) -> HostGatewayError {
    HostGatewayError::Certificate {
        path: path.to_owned(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use router_config::RouterConfig;
    use serde_json::json;

    fn config(port: u16, certificate: &Path, key: &Path) -> RouterConfig {
        serde_json::from_value(json!({
            "apiVersion": "switchyard.dev/router/v1alpha1",
            "kind": "RouterConfiguration",
            "metadata": { "deployment": "demo" },
            "spec": {
                "snapshot": {
                    "id": "host-1",
                    "version": 1,
                    "transitions": {
                        "http": { "strategy": "close" },
                        "https": { "strategy": "close" },
                        "websocket": { "strategy": "close" },
                        "grpc": { "strategy": "close" },
                        "tcp": { "strategy": "close" }
                    }
                },
                "listeners": [{
                    "consumer": "gateway",
                    "bind": { "host": "127.0.0.1", "port": port },
                    "protocol": "https",
                    "tls": { "certificate": certificate, "privateKey": key },
                    "destinations": [{ "kind": "custom_domain", "slot": "ui", "domain": "ui.demo.localhost" }]
                }],
                "providers": [{
                    "id": "ui",
                    "endpoint": { "protocol": "http", "host": "127.0.0.1", "port": 31000 }
                }],
                "groups": [],
                "bindings": [],
                "routes": [{ "consumer": "gateway", "slot": "ui", "provider": "ui" }],
                "browserRoutes": [],
                "identity": { "explicitHeader": "X-Switchyard-Route", "stripBeforeForwarding": true }
            }
        }))
        .unwrap()
    }

    fn free_port() -> u16 {
        TcpListener::bind(("127.0.0.1", 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    #[test]
    fn preflight_reports_an_occupied_port_without_writing_certificates() {
        let directory = tempfile::tempdir().unwrap();
        let occupied = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = occupied.local_addr().unwrap().port();
        let certificate = directory.path().join("host.pem");
        let key = directory.path().join("host-key.pem");
        let error = preflight(&config(port, &certificate, &key)).unwrap_err();
        assert!(matches!(error, HostGatewayError::PortConflict { .. }));
        assert!(!certificate.exists());
        assert!(!key.exists());
    }

    #[test]
    fn preflight_rejects_a_domain_claimed_by_different_slots() {
        let directory = tempfile::tempdir().unwrap();
        let certificate = directory.path().join("host.pem");
        let key = directory.path().join("host-key.pem");
        let mut config = config(free_port(), &certificate, &key);
        let mut listener = config.spec.listeners[0].clone();
        listener.bind.port = free_port();
        listener.destinations[0] = ListenerDestination::CustomDomain {
            slot: router_config::RouteSlotId::from("ui-two"),
            domain: "UI.DEMO.LOCALHOST.".into(),
        };
        config.spec.listeners.push(listener);
        let mut provider = config.spec.providers[0].clone();
        provider.id = router_config::ComponentId::from("ui-two");
        config.spec.providers.push(provider);
        config.spec.routes.push(router_config::Route {
            consumer: router_config::InstanceId::from("gateway"),
            slot: router_config::RouteSlotId::from("ui-two"),
            provider: router_config::ComponentId::from("ui-two"),
        });

        assert!(matches!(
            preflight(&config),
            Err(HostGatewayError::DomainConflict { .. })
        ));
    }

    #[test]
    fn managed_certificates_are_secure_renewable_and_cleanable() {
        let directory = tempfile::tempdir().unwrap();
        let certificate = directory.path().join("host.pem");
        let key = directory.path().join("host-key.pem");
        let config = config(0, &certificate, &key);

        let first = ensure_certificates(&config).unwrap();
        assert_eq!(first.generated, std::slice::from_ref(&certificate));
        assert_eq!(
            fs::metadata(&key).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(
            fs::read_to_string(&certificate)
                .unwrap()
                .contains("BEGIN CERTIFICATE")
        );

        let marker_path = marker_path(&certificate);
        let mut marker = read_marker(&marker_path).unwrap().unwrap();
        marker.not_after_unix = 0;
        fs::write(&marker_path, serde_json::to_vec(&marker).unwrap()).unwrap();
        let renewed = ensure_certificates(&config).unwrap();
        assert_eq!(renewed.renewed, std::slice::from_ref(&certificate));

        assert_eq!(
            cleanup_certificates(&config).unwrap(),
            std::slice::from_ref(&certificate)
        );
        assert!(!certificate.exists());
        assert!(!key.exists());
        assert!(!marker_path.exists());
    }

    #[test]
    fn external_certificates_are_never_overwritten_or_removed() {
        let directory = tempfile::tempdir().unwrap();
        let certificate = directory.path().join("external.pem");
        let key = directory.path().join("external-key.pem");
        fs::write(&certificate, "certificate").unwrap();
        fs::write(&key, "key").unwrap();
        let config = config(0, &certificate, &key);

        let report = ensure_certificates(&config).unwrap();
        assert_eq!(report.external, std::slice::from_ref(&certificate));
        assert!(cleanup_certificates(&config).unwrap().is_empty());
        assert_eq!(fs::read_to_string(certificate).unwrap(), "certificate");
    }

    #[test]
    fn managed_proxy_credentials_are_private_and_owned() {
        let directory = tempfile::tempdir().unwrap();
        let certificate = directory.path().join("host.pem");
        let key = directory.path().join("host-key.pem");
        let credential = directory.path().join("profile.credential");
        let mut config = config(0, &certificate, &key);
        config.spec.listeners[0].proxy_authentication = Some(router_config::ProxyAuthentication {
            scheme: router_config::ProxyAuthenticationScheme::Basic,
            credential_file: credential.clone(),
        });

        assert_eq!(
            ensure_proxy_credentials(&config).unwrap(),
            std::slice::from_ref(&credential)
        );
        let token = fs::read_to_string(&credential).unwrap();
        assert_eq!(token.len(), 64);
        assert_eq!(
            fs::metadata(&credential).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(ensure_proxy_credentials(&config).unwrap().is_empty());
        assert_eq!(
            cleanup_proxy_credentials(&config).unwrap(),
            std::slice::from_ref(&credential)
        );
        assert!(!credential.exists());
    }

    #[test]
    fn copied_markers_cannot_delete_different_configured_files() {
        let directory = tempfile::tempdir().unwrap();
        let managed_certificate = directory.path().join("managed.pem");
        let managed_key = directory.path().join("managed-key.pem");
        let managed = config(1, &managed_certificate, &managed_key);
        ensure_certificates(&managed).unwrap();

        let external_certificate = directory.path().join("external.pem");
        let external_key = directory.path().join("external-key.pem");
        fs::write(&external_certificate, "external certificate").unwrap();
        fs::write(&external_key, "external key").unwrap();
        fs::copy(
            marker_path(&managed_certificate),
            marker_path(&external_certificate),
        )
        .unwrap();
        fs::set_permissions(
            marker_path(&external_certificate),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let external = config(1, &external_certificate, &external_key);

        assert!(cleanup_certificates(&external).is_err());
        assert_eq!(
            fs::read_to_string(external_certificate).unwrap(),
            "external certificate"
        );
        assert_eq!(fs::read_to_string(external_key).unwrap(), "external key");

        let managed_credential = directory.path().join("managed.credential");
        let mut managed_proxy = managed.clone();
        managed_proxy.spec.listeners[0].proxy_authentication =
            Some(router_config::ProxyAuthentication {
                scheme: router_config::ProxyAuthenticationScheme::Basic,
                credential_file: managed_credential.clone(),
            });
        ensure_proxy_credentials(&managed_proxy).unwrap();
        let external_credential = directory.path().join("external.credential");
        fs::write(&external_credential, "external-token").unwrap();
        fs::set_permissions(&external_credential, fs::Permissions::from_mode(0o600)).unwrap();
        fs::copy(
            credential_marker_path(&managed_credential),
            credential_marker_path(&external_credential),
        )
        .unwrap();
        fs::set_permissions(
            credential_marker_path(&external_credential),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let mut external_proxy = external;
        external_proxy.spec.listeners[0].proxy_authentication =
            Some(router_config::ProxyAuthentication {
                scheme: router_config::ProxyAuthenticationScheme::Basic,
                credential_file: external_credential.clone(),
            });
        assert!(cleanup_proxy_credentials(&external_proxy).is_err());
        assert_eq!(
            fs::read_to_string(external_credential).unwrap(),
            "external-token"
        );
    }

    #[test]
    fn credentials_match_pingora_file_and_token_rules() {
        let directory = tempfile::tempdir().unwrap();
        let certificate = directory.path().join("host.pem");
        let key = directory.path().join("host-key.pem");
        let credential = directory.path().join("profile.credential");
        let mut config = config(1, &certificate, &key);
        config.spec.listeners[0].proxy_authentication = Some(router_config::ProxyAuthentication {
            scheme: router_config::ProxyAuthenticationScheme::Basic,
            credential_file: credential.clone(),
        });
        fs::write(&credential, b"first\nsecond").unwrap();
        fs::set_permissions(&credential, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(ensure_proxy_credentials(&config).is_err());

        fs::write(&credential, b"one-token\r\n").unwrap();
        fs::set_permissions(&credential, fs::Permissions::from_mode(0o640)).unwrap();
        assert!(ensure_proxy_credentials(&config).is_err());
        fs::set_permissions(&credential, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(ensure_proxy_credentials(&config).unwrap().is_empty());
    }

    #[test]
    fn managed_files_reject_symlinked_parent_directories() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let linked = directory.path().join("linked");
        symlink(outside.path(), &linked).unwrap();
        let certificate = linked.join("host.pem");
        let key = linked.join("host-key.pem");
        let config = config(1, &certificate, &key);

        assert!(ensure_certificates(&config).is_err());
        assert!(!outside.path().join("host.pem").exists());
        assert!(!outside.path().join("host-key.pem").exists());
    }
}
