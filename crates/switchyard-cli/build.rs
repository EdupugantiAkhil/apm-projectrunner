use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/index");
    let describe = Command::new("git")
        .args(["describe", "--always", "--dirty"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=SWITCHYARD_GIT_DESCRIBE={describe}");
}
