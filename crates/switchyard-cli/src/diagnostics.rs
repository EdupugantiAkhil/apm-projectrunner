use std::{
    collections::BTreeMap,
    env, fs,
    io::{self, BufRead, BufReader, Write},
    os::unix::{
        fs::{OpenOptionsExt, PermissionsExt},
        net::UnixStream,
    },
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde_json::{Map, Value, json};

use crate::runtime::DockerRuntime;

const REDACTED: &str = "[REDACTED]";
const REDACTED_ENVIRONMENT: &str = "[REDACTED:environment]";
const LOG_TAIL_LINES: usize = 200;
const MAX_FILE_BYTES: u64 = 1024 * 1024;

pub fn write_bundle(
    workspace_root: &Path,
    deployment_path: &Path,
    requested_output: Option<&Path>,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let authored = switchyard_planner::load_bundle(deployment_path)?;
    let deployment = authored.metadata.name.clone();
    let default_deployment_name = safe_file_component(&deployment);
    let planned = switchyard_planner::plan(&authored);
    let (plan, definition_hash, validation_diagnostics) = match planned {
        Ok(plan) => {
            let hash = Value::String(plan.definition_hash.clone());
            (Some(plan), hash, json!([]))
        }
        Err(diagnostics) => (None, Value::Null, serde_json::to_value(diagnostics)?),
    };

    let daemon_status = switchyard_daemon::client::daemon_status(workspace_root)
        .ok()
        .flatten();
    let state = match daemon_status {
        Some(status) => {
            let detail =
                match switchyard_daemon::client::deployment_detail(workspace_root, &deployment) {
                    Ok(Some(detail)) => serde_json::to_value(detail)?,
                    Ok(None) => json!({"unavailable": "daemon became unreachable"}),
                    Err(error) => json!({"unavailable": error.to_string()}),
                };
            json!({"source": "daemon", "status": status, "detail": detail})
        }
        None => json!({
            "source": "files",
            "generated": collect_json_files(
                &workspace_root.join(".switchyard/generated").join(&deployment),
            ),
            "runtime": collect_json_files(
                &workspace_root.join(".switchyard/run").join(&deployment),
            ),
        }),
    };

    let resources = match DockerRuntime::default().discover(&deployment) {
        Ok(resources) => Value::Array(
            resources
                .into_iter()
                .map(|resource| {
                    json!({
                        "kind": resource.kind.to_string(),
                        "id": resource.id,
                        "name": resource.name,
                        "labels": resource.labels,
                        "state": resource.state,
                    })
                })
                .collect(),
        ),
        Err(error) => json!({"unavailable": error.to_string()}),
    };

    let run_dir = workspace_root.join(".switchyard/run").join(&deployment);
    let mut report = json!({
        "schemaVersion": "switchyard.dev/diagnostics/v1alpha1",
        "createdAtUnixSeconds": now_seconds(),
        "tool": {
            "version": env!("CARGO_PKG_VERSION"),
            "gitDescribe": env!("SWITCHYARD_GIT_DESCRIBE"),
        },
        "system": {
            "os": env::consts::OS,
            "arch": env::consts::ARCH,
            "kernel": command_text("uname", &["-sr"], workspace_root),
            "docker": command_observation("docker", &["version", "--format", "json"], workspace_root),
            "compose": command_observation("docker", &["compose", "version", "--format", "json"], workspace_root),
        },
        "deployment": {
            "name": deployment,
            "definitionHash": definition_hash,
            "validationDiagnostics": validation_diagnostics,
        },
        "state": state,
        "logs": {
            "hostGateway": tail_file(&run_dir.join("host-gateway.log"), LOG_TAIL_LINES),
        },
        "routerEvents": collect_router_events(workspace_root, &run_dir, plan.as_ref()),
        "resourceObservations": resources,
    });

    let redactor = Redactor::from_process(workspace_root);
    redactor.redact(&mut report);

    let output = requested_output.map(Path::to_owned).unwrap_or_else(|| {
        PathBuf::from(format!(
            "switchyard-diagnostics-{}-{}.json",
            default_deployment_name,
            now_seconds()
        ))
    });
    if fs::symlink_metadata(&output).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(format!(
            "diagnostics output must not be a symbolic link: {}",
            output.display()
        )
        .into());
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&output)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    serde_json::to_writer_pretty(&mut file, &report)?;
    file.write_all(b"\n")?;
    Ok(output)
}

fn command_text(program: &str, arguments: &[&str], directory: &Path) -> Value {
    match Command::new(program)
        .args(arguments)
        .current_dir(directory)
        .output()
    {
        Ok(output) if output.status.success() => {
            Value::String(String::from_utf8_lossy(&output.stdout).trim().to_owned())
        }
        Ok(output) => json!({
            "unavailable": String::from_utf8_lossy(&output.stderr).trim(),
            "exitCode": output.status.code(),
        }),
        Err(error) => json!({"unavailable": error.to_string()}),
    }
}

fn command_observation(program: &str, arguments: &[&str], directory: &Path) -> Value {
    let value = command_text(program, arguments, directory);
    match value {
        Value::String(text) => serde_json::from_str(&text).unwrap_or(Value::String(text)),
        other => other,
    }
}

fn collect_json_files(root: &Path) -> Value {
    let mut files = BTreeMap::new();
    collect_json_files_at(root, root, &mut files);
    serde_json::to_value(files).unwrap_or_else(|_| json!({}))
}

fn collect_json_files_at(root: &Path, current: &Path, files: &mut BTreeMap<String, Value>) {
    let Ok(entries) = fs::read_dir(current) else {
        return;
    };
    let mut entries = entries.flatten().collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            collect_json_files_at(root, &path, files);
        } else if metadata.is_file()
            && metadata.len() <= MAX_FILE_BYTES
            && path
                .extension()
                .is_some_and(|extension| extension == "json")
        {
            let relative = path.strip_prefix(root).unwrap_or(&path);
            let value = fs::read_to_string(&path)
                .ok()
                .and_then(|contents| serde_json::from_str(&contents).ok())
                .unwrap_or_else(|| json!({"unavailable": "invalid or unreadable JSON"}));
            files.insert(relative.display().to_string(), value);
        }
    }
}

fn tail_file(path: &Path, limit: usize) -> Value {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return Value::Null;
    };
    if !metadata.file_type().is_file() {
        return json!({"unavailable": "not a regular file"});
    }
    let Ok(contents) = fs::read_to_string(path) else {
        return json!({"unavailable": "unreadable log"});
    };
    Value::Array(
        contents
            .lines()
            .rev()
            .take(limit)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|line| Value::String(line.to_owned()))
            .collect(),
    )
}

fn collect_router_events(
    workspace_root: &Path,
    run_dir: &Path,
    plan: Option<&switchyard_planner::Plan>,
) -> Value {
    let Some(token) = env::var_os("SWITCHYARD_ROUTER_TOKEN") else {
        return json!({"unavailable": "SWITCHYARD_ROUTER_TOKEN is not set"});
    };
    let Some(token) = token.to_str() else {
        return json!({"unavailable": "SWITCHYARD_ROUTER_TOKEN is not valid UTF-8"});
    };
    let mut sockets = BTreeMap::from([("host".to_owned(), run_dir.join("host.socket"))]);
    if let Some(plan) = plan {
        for (name, sidecar) in &plan.sidecars {
            sockets.insert(name.clone(), workspace_root.join(&sidecar.admin_socket));
        }
    }
    Value::Object(
        sockets
            .into_iter()
            .map(|(name, socket)| {
                let value = router_events(&socket, token)
                    .unwrap_or_else(|error| json!({"unavailable": error.to_string()}));
                (name, value)
            })
            .collect(),
    )
}

fn router_events(socket: &Path, token: &str) -> io::Result<Value> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(Duration::from_secs(1)))?;
    stream.set_write_timeout(Some(Duration::from_secs(1)))?;
    serde_json::to_writer(&mut stream, &json!({"token": token, "operation": "events"}))?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    let mut response = String::new();
    BufReader::new(stream).read_line(&mut response)?;
    let response: Value = serde_json::from_str(&response)?;
    Ok(response.get("result").cloned().unwrap_or(response))
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn safe_file_component(value: &str) -> String {
    let value = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    if value.is_empty() {
        "deployment".into()
    } else {
        value
    }
}

struct Redactor {
    environment_values: Vec<String>,
}

impl Redactor {
    fn from_process(workspace_root: &Path) -> Self {
        // Only values of credential-looking variable names are treated as secrets.
        // Replacing every process environment value would also erase benign values
        // like $HOME from every path (destroying diagnosability) and mangle
        // arbitrary text whenever a variable holds a common short word.
        let mut environment_values = env::vars_os()
            .filter_map(|(name, value)| {
                let name = name.into_string().ok()?;
                switchyard_planner::credential_like_key(&name).then_some(value.into_string().ok()?)
            })
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        if let Ok(contents) = fs::read_to_string(workspace_root.join(".switchyard/daemon.json")) {
            if let Ok(discovery) = serde_json::from_str::<Value>(&contents) {
                if let Some(token) = discovery.get("token").and_then(Value::as_str) {
                    environment_values.push(token.to_owned());
                }
            }
        }
        environment_values.sort_by_key(|value| std::cmp::Reverse(value.len()));
        environment_values.dedup();
        Self { environment_values }
    }

    #[cfg(test)]
    fn with_environment_values(values: &[&str]) -> Self {
        Self {
            environment_values: values.iter().map(|value| (*value).to_owned()).collect(),
        }
    }

    fn redact(&self, value: &mut Value) {
        match value {
            Value::Object(object) => self.redact_object(object),
            Value::Array(values) => {
                for value in values {
                    self.redact(value);
                }
            }
            Value::String(string) => self.redact_string(string),
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
    }

    fn redact_object(&self, object: &mut Map<String, Value>) {
        for (key, value) in object {
            if switchyard_planner::credential_like_key(key) {
                *value = Value::String(REDACTED.into());
            } else {
                self.redact(value);
            }
        }
    }

    fn redact_string(&self, string: &mut String) {
        if switchyard_planner::redact_event_line(string) == REDACTED {
            *string = REDACTED.into();
            return;
        }
        for secret in &self.environment_values {
            if string == secret {
                *string = REDACTED_ENVIRONMENT.into();
                return;
            }
            if secret.len() >= 4 && string.contains(secret) {
                *string = string.replace(secret, REDACTED_ENVIRONMENT);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recursive_redaction_removes_planted_secrets() {
        let environment_secret = "env-value-9a27";
        let router_secret = "router-value-61bf";
        let daemon_secret = "daemon-value-22cc";
        let password = "password-value-b790";
        let mut report = json!({
            "nested": {
                "database_password": password,
                "ordinary": format!("prefix {environment_secret} suffix"),
                "router": router_secret,
                "daemon": daemon_secret,
            },
            "logs": [
                format!("Authorization: Bearer {daemon_secret}"),
                "healthy request"
            ],
        });
        Redactor::with_environment_values(&[environment_secret, router_secret, daemon_secret])
            .redact(&mut report);

        let output = serde_json::to_string(&report).unwrap();
        for secret in [environment_secret, router_secret, daemon_secret, password] {
            assert!(
                !output.contains(secret),
                "planted secret survived: {secret}"
            );
        }
        assert!(output.contains(REDACTED));
        assert!(output.contains(REDACTED_ENVIRONMENT));
        assert!(output.contains("healthy request"));
    }
}
