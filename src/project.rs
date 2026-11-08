//! Which project a container belongs to, and what to call it.

use std::path::Path;

use crate::docker::ContainerLookup;
use crate::store::{Project, Store};
use crate::text::prefix;

/// Looks the container up in the store; on a miss, asks docker and caches the answer.
/// A container's labels do not change, so a cached entry is final.
pub fn resolve(store: &Store, docker: &dyn ContainerLookup, id: &str) -> Option<Project> {
    match store.find_project(id) {
        Ok(Some(project)) => return Some(project),
        Ok(None) => {}
        Err(e) => eprintln!("failed to look up project {id}: {e}"),
    }
    let info = docker.inspect(id)?;
    if let Err(e) = store.upsert_project(&info.id, info.local_folder.as_deref()) {
        eprintln!("failed to store project {}: {e}", info.id);
    }
    Some(Project {
        container_id: info.id,
        local_folder: info.local_folder,
    })
}

/// The folder name from the devcontainer label, else a short container ID, else `default`.
pub fn display_name(
    project: Option<&Project>,
    container_id: Option<&str>,
    default: &str,
) -> String {
    project
        .and_then(|p| p.local_folder.as_deref())
        .and_then(folder_name)
        .or_else(|| container_id.map(|id| prefix(id, 12).to_string()))
        .unwrap_or_else(|| default.to_string())
}

pub fn folder_name(path: &str) -> Option<String> {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docker::ContainerInfo;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const FULL: &str = "43afb2711be74f19612a17a789fdb3dd6ca1ded7737de4a8b1124035f87e41b2";

    struct FakeDocker {
        calls: AtomicUsize,
        folder: Option<&'static str>,
    }

    impl ContainerLookup for FakeDocker {
        fn inspect(&self, id: &str) -> Option<ContainerInfo> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            FULL.starts_with(id).then(|| ContainerInfo {
                id: FULL.into(),
                local_folder: self.folder.map(Into::into),
            })
        }
    }

    fn docker(folder: Option<&'static str>) -> FakeDocker {
        FakeDocker {
            calls: AtomicUsize::new(0),
            folder,
        }
    }

    #[test]
    fn asks_docker_once_then_uses_the_store() {
        let store = Store::open_in_memory().unwrap();
        let docker = docker(Some("/Users/me/src/api"));
        for _ in 0..3 {
            let p = resolve(&store, &docker, &FULL[..12]).unwrap();
            assert_eq!(p.container_id, FULL);
            assert_eq!(p.local_folder.as_deref(), Some("/Users/me/src/api"));
        }
        assert_eq!(docker.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn caches_containers_without_a_label() {
        let store = Store::open_in_memory().unwrap();
        let docker = docker(None);
        assert!(
            resolve(&store, &docker, FULL)
                .unwrap()
                .local_folder
                .is_none()
        );
        assert!(resolve(&store, &docker, FULL).is_some());
        assert_eq!(docker.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn unknown_container() {
        let store = Store::open_in_memory().unwrap();
        let docker = docker(None);
        assert!(resolve(&store, &docker, "ffff").is_none());
        // Misses are not cached; the container may appear later.
        assert!(resolve(&store, &docker, "ffff").is_none());
        assert_eq!(docker.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn display_names() {
        let p = |folder: Option<&str>| Project {
            container_id: "c".into(),
            local_folder: folder.map(Into::into),
        };
        assert_eq!(
            display_name(Some(&p(Some("/Users/me/src/web"))), None, "d"),
            "web"
        );
        assert_eq!(
            display_name(Some(&p(None)), Some("43afb2711be74f19612a"), "d"),
            "43afb2711be7"
        );
        assert_eq!(display_name(None, None, "Claude Code"), "Claude Code");
    }
}
