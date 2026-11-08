//! Resolves container IDs through `docker inspect`.

use std::process::Command;
use std::time::Duration;

use crate::process;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerInfo {
    pub id: String,
    pub local_folder: Option<String>,
}

pub trait ContainerLookup: Send + Sync {
    fn inspect(&self, id: &str) -> Option<ContainerInfo>;
}

/// The devcontainer CLI labels each container with the host folder it was started from.
const FORMAT: &str = "{{.Id}}\t{{index .Config.Labels \"devcontainer.local_folder\"}}";

pub struct DockerCli {
    program: String,
    timeout: Duration,
}

impl DockerCli {
    pub fn new(program: impl Into<String>, timeout: Duration) -> Self {
        Self {
            program: program.into(),
            timeout,
        }
    }
}

impl ContainerLookup for DockerCli {
    fn inspect(&self, id: &str) -> Option<ContainerInfo> {
        if !is_container_id(id) {
            return None;
        }
        let mut command = Command::new(&self.program);
        command.args(["inspect", "--type", "container", "--format", FORMAT, id]);
        let out = match process::run(command, self.timeout) {
            Ok(out) => out,
            Err(e) => {
                eprintln!("docker inspect {id}: {e}");
                return None;
            }
        };
        if !out.status.success() {
            eprintln!("docker inspect {id}: {}", out.stderr.trim());
            return None;
        }
        parse(&out.stdout)
    }
}

fn parse(stdout: &str) -> Option<ContainerInfo> {
    let line = stdout.lines().next()?;
    let (id, folder) = line.split_once('\t').unwrap_or((line, ""));
    // An unset label prints as "<no value>".
    let folder = folder.trim();
    let local_folder = (!folder.is_empty() && folder != "<no value>").then(|| folder.to_string());
    is_container_id(id).then(|| ContainerInfo {
        id: id.to_string(),
        local_folder,
    })
}

/// Lowercase hex, 1 to 64 characters. Keeps arbitrary header values out of argv.
pub fn is_container_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAKE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake-docker.sh");
    pub const FULL: &str = "aaaa000000000000000000000000000000000000000000000000000000000001";

    #[test]
    fn container_ids() {
        assert!(is_container_id("43afb2711be7"));
        assert!(is_container_id(FULL));
        assert!(!is_container_id(""));
        assert!(!is_container_id("-f"));
        assert!(!is_container_id("ABC"));
        assert!(!is_container_id(&"a".repeat(65)));
    }

    #[test]
    fn parse_output() {
        assert_eq!(
            parse(&format!("{FULL}\t/Users/me/src/api\n")),
            Some(ContainerInfo {
                id: FULL.into(),
                local_folder: Some("/Users/me/src/api".into())
            })
        );
        assert_eq!(
            parse(&format!("{FULL}\t<no value>\n"))
                .unwrap()
                .local_folder,
            None
        );
        assert_eq!(parse("garbage\n"), None);
        assert_eq!(parse(""), None);
    }

    #[test]
    fn inspect_with_fake_docker() {
        let docker = DockerCli::new(FAKE, Duration::from_secs(5));
        let info = docker.inspect("aaaa").unwrap();
        assert_eq!(info.id, FULL);
        assert_eq!(info.local_folder.as_deref(), Some("/Users/me/src/api"));
        assert!(docker.inspect("bbbb").is_none());
        assert!(docker.inspect("--help").is_none());
    }
}
