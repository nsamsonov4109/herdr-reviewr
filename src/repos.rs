//! The repositories one pane reviews: the repository it starts in, or the repositories directly
//! inside a folder no repository holds.

use std::path::Path;
use std::sync::Arc;

/// The repositories under review by folder name, sorted; empty when the root is the one repository.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Repos {
    names: Arc<[String]>,
}

impl Repos {
    /// The repositories directly inside `root`: each folder holding a `.git`, hidden ones skipped.
    pub fn discover(root: &Path) -> Self {
        let Ok(dir) = std::fs::read_dir(root) else { return Self::default() };
        // A link is skipped: its files would live outside the folder the watcher watches.
        let mut names: Vec<String> = dir
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|t| t.is_dir()))
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| !name.starts_with('.') && root.join(name).join(".git").exists())
            .collect();
        names.sort();
        Self { names: names.into() }
    }

    /// A folder of the repositories `names`, for tests.
    #[cfg(test)]
    pub(crate) fn of(names: &[&str]) -> Self {
        let mut names: Vec<String> = names.iter().map(|n| (*n).to_string()).collect();
        names.sort();
        Self { names: names.into() }
    }

    /// Whether the root is a folder of repositories rather than one repository.
    pub fn is_folder(&self) -> bool {
        !self.names.is_empty()
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    pub fn contains(&self, name: &str) -> bool {
        self.names.binary_search_by(|n| n.as_str().cmp(name)).is_ok()
    }

    /// The repository a folder-relative `path` lies in and the path inside it; `None` outside all.
    pub fn locate<'a>(&self, path: &'a str) -> Option<(&str, &'a str)> {
        let (name, inner) = path.split_once('/')?;
        let at = self.names.binary_search_by(|n| n.as_str().cmp(name)).ok()?;
        (!inner.is_empty()).then(|| (self.names[at].as_str(), inner))
    }
}

/// `inner`, a path inside the repository `name`, spelled from the folder.
pub fn join(name: &str, inner: &str) -> String {
    format!("{name}/{inner}")
}

/// `path`, spelled from the folder, as the repository `name` spells it; `None` outside it.
pub fn inside<'a>(name: &str, path: &'a str) -> Option<&'a str> {
    path.strip_prefix(name)?.strip_prefix('/').filter(|inner| !inner.is_empty())
}

/// Run `f` over `items` on a few threads, results in order: each repository's git runs on its own.
pub fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let threads = std::thread::available_parallelism().map_or(1, usize::from).clamp(1, 8);
    if items.len() < 2 || threads == 1 {
        return items.iter().map(f).collect();
    }
    let next = AtomicUsize::new(0);
    let mut done: Vec<(usize, R)> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads.min(items.len()))
            .map(|_| {
                scope.spawn(|| {
                    let mut out = Vec::new();
                    loop {
                        let at = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = items.get(at) else { break };
                        out.push((at, f(item)));
                    }
                    out
                })
            })
            .collect();
        // A panic in one repository's build reaches the caller, as it would on one thread.
        workers
            .into_iter()
            .flat_map(|w| w.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
            .collect()
    });
    done.sort_by_key(|(at, _)| *at);
    done.into_iter().map(|(_, r)| r).collect()
}

#[cfg(test)]
mod tests {
    use super::{Repos, join, par_map};

    #[test]
    fn a_folder_holds_the_repositories_directly_inside_it_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for repo in ["web", "api"] {
            std::fs::create_dir_all(root.join(repo).join(".git")).unwrap();
        }
        // A linked worktree's `.git` is a file.
        std::fs::create_dir(root.join("tool")).unwrap();
        std::fs::write(root.join("tool/.git"), "gitdir: /elsewhere\n").unwrap();
        std::fs::create_dir_all(root.join("docs/inner/.git")).unwrap();
        std::fs::create_dir_all(root.join(".hidden/.git")).unwrap();
        std::fs::write(root.join("notes.txt"), "x").unwrap();
        let repos = Repos::discover(root);
        assert_eq!(repos.names(), ["api", "tool", "web"], "sorted, direct children with a .git");
        assert!(repos.is_folder());
    }

    #[test]
    fn a_folder_with_no_repository_is_no_folder_of_repositories() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("plain")).unwrap();
        assert!(!Repos::discover(dir.path()).is_folder());
        assert!(!Repos::discover(&dir.path().join("missing")).is_folder());
    }

    #[test]
    fn a_path_locates_its_repository_by_its_first_component() {
        let repos = Repos::of(&["api", "web"]);
        assert_eq!(repos.locate("api/src/main.rs"), Some(("api", "src/main.rs")));
        assert_eq!(repos.locate("web/a"), Some(("web", "a")));
        assert_eq!(repos.locate("api"), None, "the repository itself is no path inside it");
        assert_eq!(repos.locate("notes.txt"), None, "a file beside the repositories");
        assert_eq!(repos.locate("apix/a"), None, "a prefix of a name is not the name");
        assert_eq!(join("api", "src/main.rs"), "api/src/main.rs");
    }

    #[test]
    fn par_map_keeps_the_order_of_its_items() {
        let items: Vec<usize> = (0..50).collect();
        assert_eq!(par_map(&items, |n| n * 2), items.iter().map(|n| n * 2).collect::<Vec<_>>());
    }
}
