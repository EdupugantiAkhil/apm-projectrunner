use std::io;
#[cfg(target_os = "linux")]
use std::process::{Command, Output};

#[cfg(target_os = "linux")]
use router_tcp::TRANSPARENT_BYPASS_MARK;

#[cfg(target_os = "linux")]
const CHAIN: &str = "SWITCHYARD_OUT";

pub struct InterceptionGuard {
    installed: bool,
}

impl InterceptionGuard {
    pub fn install(port: u16) -> io::Result<Self> {
        #[cfg(not(target_os = "linux"))]
        {
            let _ = port;
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "transparent localhost interception requires Linux",
            ))
        }

        #[cfg(target_os = "linux")]
        {
            install_family("iptables", "127.0.0.0/8", Some("127.0.0.11/32"), port)?;
            if let Err(error) = install_family("ip6tables", "::1/128", None, port) {
                cleanup_family("iptables", "127.0.0.0/8", Some("127.0.0.11/32"));
                return Err(error);
            }
            Ok(Self { installed: true })
        }
    }

    pub fn remove(&mut self) {
        if !self.installed {
            return;
        }
        #[cfg(target_os = "linux")]
        {
            cleanup_family("ip6tables", "::1/128", None);
            cleanup_family("iptables", "127.0.0.0/8", Some("127.0.0.11/32"));
        }
        self.installed = false;
    }
}

impl Drop for InterceptionGuard {
    fn drop(&mut self) {
        self.remove();
    }
}

#[cfg(target_os = "linux")]
fn install_family(
    program: &str,
    destination: &str,
    excluded_destination: Option<&str>,
    port: u16,
) -> io::Result<()> {
    let _ = run(program, &["-t", "nat", "-N", CHAIN]);
    checked(program, &["-t", "nat", "-F", CHAIN])?;
    checked(
        program,
        &[
            "-t",
            "nat",
            "-A",
            CHAIN,
            "-m",
            "mark",
            "--mark",
            &format!("{TRANSPARENT_BYPASS_MARK:#x}"),
            "-j",
            "RETURN",
        ],
    )?;
    checked(
        program,
        &[
            "-t",
            "nat",
            "-A",
            CHAIN,
            "-p",
            "tcp",
            "-j",
            "REDIRECT",
            "--to-ports",
            &port.to_string(),
        ],
    )?;
    if let Some(excluded) = excluded_destination {
        let exclusion = [
            "-t", "nat", "-C", "OUTPUT", "-p", "tcp", "-d", excluded, "-j", "RETURN",
        ];
        if !run(program, &exclusion)?.status.success() {
            let mut addition = exclusion;
            addition[2] = "-A";
            checked(program, &addition)?;
        }
    }
    let mut check = vec!["-t", "nat", "-C", "OUTPUT", "-p", "tcp", "-d", destination];
    check.extend(["-j", CHAIN]);
    if !run(program, &check)?.status.success() {
        let mut jump = check;
        jump[2] = "-A";
        checked(program, &jump)?;
    }
    let inbound_check = ["-t", "nat", "-C", "PREROUTING", "-p", "tcp", "-j", CHAIN];
    if !run(program, &inbound_check)?.status.success() {
        let mut inbound_jump = inbound_check;
        inbound_jump[2] = "-A";
        checked(program, &inbound_jump)?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn cleanup_family(program: &str, destination: &str, excluded_destination: Option<&str>) {
    let _ = run(
        program,
        &["-t", "nat", "-D", "PREROUTING", "-p", "tcp", "-j", CHAIN],
    );
    let mut jump = vec!["-t", "nat", "-D", "OUTPUT", "-p", "tcp", "-d", destination];
    jump.extend(["-j", CHAIN]);
    let _ = run(program, &jump);
    if let Some(excluded) = excluded_destination {
        let _ = run(
            program,
            &[
                "-t", "nat", "-D", "OUTPUT", "-p", "tcp", "-d", excluded, "-j", "RETURN",
            ],
        );
    }
    let _ = run(program, &["-t", "nat", "-F", CHAIN]);
    let _ = run(program, &["-t", "nat", "-X", CHAIN]);
}

#[cfg(target_os = "linux")]
fn checked(program: &str, arguments: &[&str]) -> io::Result<()> {
    let output = run(program, arguments)?;
    if output.status.success() {
        return Ok(());
    }
    Err(io::Error::other(format!(
        "{program} {} failed: {}",
        arguments.join(" "),
        String::from_utf8_lossy(&output.stderr).trim()
    )))
}

#[cfg(target_os = "linux")]
fn run(program: &str, arguments: &[&str]) -> io::Result<Output> {
    Command::new(program).args(arguments).output()
}
