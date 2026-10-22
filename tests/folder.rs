//! A folder of repositories reviewed as one: each repository's changes under its name, each
//! diffed against its own ends and base, and a refresh re-reading only the repository it names.

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;

use herdr_reviewr::app::{App, Tab};
use herdr_reviewr::model::Scope;
use herdr_reviewr::repos::Repos;
use herdr_reviewr::wake::{Waker, channel};
use herdr_reviewr::world::{self, Refresh, WorldCompletion, WorldInput, WorldJob};

/// A folder no repository holds, with repositories made inside it.
struct Folder {
    dir: tempfile::TempDir,
}

impl Folder {
    fn new() -> Self {
        Self { dir: tempfile::tempdir().unwrap() }
    }

    fn root(&self) -> PathBuf {
        self.dir.path().to_path_buf()
    }

    /// A repository `name` on `main` holding `files`, committed.
    fn repo(&self, name: &str, files: &[(&str, &str)]) -> PathBuf {
        let at = self.dir.path().join(name);
        std::fs::create_dir_all(&at).unwrap();
        git(&at, &["init", "-q", "-b", "main"]);
        git(&at, &["config", "init.defaultBranch", "no-such-default"]);
        for (path, text) in files {
            write(&at, path, text);
        }
        commit_all(&at, "init");
        at
    }
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@herdr.test")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@herdr.test")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn write(dir: &Path, rel: &str, text: &str) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn commit_all(dir: &Path, message: &str) {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", message]);
}

/// Fake `origin/<name>` at `rev`, with no real remote.
fn origin(dir: &Path, name: &str, rev: &str) {
    let oid = git(dir, &["rev-parse", rev]);
    git(dir, &["update-ref", &format!("refs/remotes/origin/{name}"), &oid]);
}

/// A pane over the folder at `root`, seeded from `toml` when given, on `scope`.
fn app_on(root: &Path, toml: Option<&str>, scope: Scope) -> App {
    let mut app = App::new(root.to_path_buf(), Scope::Uncommitted, None);
    if let Some(toml) = toml {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.toml"), toml).unwrap();
        app.set_plugin_config(herdr_reviewr::config::plugin_config_in(dir.path()).unwrap());
    }
    app.set_repos(Repos::discover(root));
    app.reload().unwrap();
    app.set_scope(scope).unwrap();
    app
}

fn listed(app: &App) -> Vec<String> {
    app.entries.iter().map(|e| e.path.clone()).collect()
}

fn open(app: &mut App, path: &str) -> Vec<String> {
    let row = app
        .file_rows
        .iter()
        .position(|r| r.file_index().is_some_and(|i| app.entries[i].path == path))
        .expect("a file row for the path");
    app.select_file(row).unwrap();
    app.diff.rows.iter().map(|r| format!("{}{}", r.marker(), r.text())).collect()
}

#[test]
fn a_folder_lists_each_repositorys_changes_under_its_name() {
    let f = Folder::new();
    let api = f.repo("api", &[("src/a.rs", "one\n")]);
    let web = f.repo("web", &[("b.txt", "two\n")]);
    write(&api, "src/a.rs", "one\nmore\n");
    write(&api, "new.rs", "fresh\n");
    write(&web, "b.txt", "TWO\n");
    // Beside the repositories, so in none of them.
    write(&f.root(), "notes.txt", "x\n");
    let mut app = app_on(&f.root(), None, Scope::Uncommitted);
    assert_eq!(listed(&app), ["api/new.rs", "api/src/a.rs", "web/b.txt"]);
    let rows = open(&mut app, "api/src/a.rs");
    assert!(rows.contains(&"+more".to_string()), "read in api: {rows:?}");
    let rows = open(&mut app, "web/b.txt");
    assert!(rows.contains(&"-two".to_string()) && rows.contains(&"+TWO".to_string()), "{rows:?}");
    assert_eq!(open(&mut app, "api/new.rs"), ["+fresh"], "an untracked file reads raw");
}

#[test]
fn each_repository_diffs_against_the_first_default_base_it_has() {
    let f = Folder::new();
    let api = f.repo("api", &[("a.txt", "base\n")]);
    // `api` forks its feature off `dev`, which is ahead of `main`.
    git(&api, &["checkout", "-q", "-b", "dev"]);
    write(&api, "dev-only.txt", "dev\n");
    commit_all(&api, "dev work");
    origin(&api, "dev", "dev");
    origin(&api, "main", "main");
    git(&api, &["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main"]);
    git(&api, &["checkout", "-q", "-b", "feature"]);
    write(&api, "a.txt", "feature\n");
    commit_all(&api, "feature");
    // `web` has no `dev`: it keeps its default branch.
    let web = f.repo("web", &[("b.txt", "base\n")]);
    origin(&web, "main", "main");
    git(&web, &["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main"]);
    git(&web, &["checkout", "-q", "-b", "feature"]);
    write(&web, "b.txt", "feature\n");
    commit_all(&web, "feature");

    // Uncommitted work counts too: an edit on top of the commit, and a new file.
    write(&web, "b.txt", "feature, and more\n");
    write(&web, "new.txt", "fresh\n");
    let app = app_on(&f.root(), Some("default_base = [\"dev\", \"trunk\"]\n"), Scope::Branch);
    assert_eq!(
        listed(&app),
        ["api/a.txt", "web/b.txt", "web/new.txt"],
        "dev's own file is no change vs dev; the uncommitted ones are"
    );
    let base = |name: &str| app.repo_bases[name].winner.as_ref().map(|w| w.name().to_string());
    assert_eq!(base("api").as_deref(), Some("dev"));
    assert_eq!(base("web").as_deref(), Some("main"), "a name it lacks is skipped quietly");
    assert_eq!(app.repo_bases["web"].skipped, None);

    // Without the config, `api` diffs against its default and shows dev's work too.
    let app = app_on(&f.root(), None, Scope::Branch);
    assert_eq!(listed(&app), ["api/a.txt", "api/dev-only.txt", "web/b.txt", "web/new.txt"]);
}

struct Worker {
    jobs: mpsc::Sender<WorldJob>,
    done: mpsc::Receiver<WorldCompletion>,
    generation: u64,
}

impl Worker {
    fn on(root: &Path) -> Self {
        let (jobs, rx) = mpsc::channel();
        let (tx, done) = channel(&Waker::detached());
        world::spawn(root.to_path_buf(), rx, tx);
        Self { jobs, done, generation: 0 }
    }

    fn run(&mut self, input: &WorldInput, refresh: Refresh) -> Option<world::WorldSnapshot> {
        self.generation += 1;
        let job =
            WorldJob { generation: self.generation, input: input.clone(), reveal: false, refresh };
        self.jobs.send(job).unwrap();
        self.done.recv().unwrap().snapshot.map(|s| s.expect("the build succeeds"))
    }
}

fn one_repo(name: &str, paths: &[&str]) -> Refresh {
    let each = Refresh::paths(paths.iter().map(|p| (*p).to_string()));
    Refresh::Repos(BTreeMap::from([(name.to_string(), each)]))
}

#[test]
fn a_refresh_naming_one_repository_lands_what_a_full_build_would() {
    let f = Folder::new();
    let api = f.repo("api", &[("a.txt", "one\n")]);
    let web = f.repo("web", &[("b.txt", "two\n")]);
    write(&api, "a.txt", "ONE\n");
    for tab in [Tab::Changes, Tab::AllFiles] {
        let mut app = App::new(f.root(), Scope::Uncommitted, None);
        app.set_repos(Repos::discover(&f.root()));
        let mut input = app.world_input();
        input.tab = tab;
        let mut worker = Worker::on(&f.root());
        let first = worker.run(&input, Refresh::Full).expect("a first build");
        assert_eq!(first.touched, None, "a full build re-read everything");
        write(&web, "b.txt", "TWO\n");
        write(&web, "c.txt", "new\n");
        let narrow = worker.run(&input, one_repo("web", &["b.txt", "c.txt"])).expect("a re-read");
        let full = world::build(&input).unwrap();
        assert_eq!(narrow.changeset.files, full.changeset.files, "{tab:?}");
        assert_eq!(narrow.entries, full.entries, "{tab:?}");
        let touched = narrow.touched.expect("a narrow build names what it read");
        assert!(touched.iter().all(|p| p.starts_with("web/")), "{touched:?}");
        // A refresh naming nothing re-reads nothing.
        assert!(worker.run(&input, Refresh::Repos(BTreeMap::new())).is_none());
    }
}

#[test]
fn a_repository_whose_build_fails_keeps_its_last_build_and_is_named() {
    let f = Folder::new();
    let api = f.repo("api", &[("a.txt", "one\n")]);
    f.repo("web", &[("b.txt", "two\n")]);
    write(&api, "a.txt", "ONE\n");
    let mut app = App::new(f.root(), Scope::Uncommitted, None);
    app.set_repos(Repos::discover(&f.root()));
    let input = app.world_input();
    let mut worker = Worker::on(&f.root());
    let first = worker.run(&input, Refresh::Full).unwrap();
    assert_eq!(first.changeset.files.keys().collect::<Vec<_>>(), ["api/a.txt"]);
    // An index git cannot read fails every build in `api`.
    std::fs::write(api.join(".git/index"), "garbage\n").unwrap();
    let after = worker.run(&input, Refresh::Full).unwrap();
    assert_eq!(after.changeset.files, first.changeset.files, "its last build stands in");
    assert_eq!(after.failed.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), ["api"]);
}

#[test]
fn a_repository_that_leaves_the_folder_takes_its_files_with_it() {
    let f = Folder::new();
    let api = f.repo("api", &[("a.txt", "one\n")]);
    let web = f.repo("web", &[("b.txt", "two\n")]);
    write(&api, "a.txt", "ONE\n");
    write(&web, "b.txt", "TWO\n");
    let mut app = App::new(f.root(), Scope::Uncommitted, None);
    app.set_repos(Repos::discover(&f.root()));
    let mut worker = Worker::on(&f.root());
    worker.run(&app.world_input(), Refresh::Full).unwrap();
    std::fs::remove_dir_all(&web).unwrap();
    app.set_repos(Repos::discover(&f.root()));
    let after = worker.run(&app.world_input(), Refresh::Repos(BTreeMap::new())).unwrap();
    assert_eq!(after.changeset.files.keys().collect::<Vec<_>>(), ["api/a.txt"]);
}

/// A folder whose `api` forks `feature` off `dev` and whose `web` forks it off `main`, both
/// with a commit on the feature.
fn forked() -> (Folder, PathBuf, PathBuf) {
    let f = Folder::new();
    let api = f.repo("api", &[("a.txt", "base\n")]);
    git(&api, &["branch", "dev"]);
    for (repo, from) in [(&api, "dev"), (&f.repo("web", &[("b.txt", "base\n")]), "main")] {
        origin(repo, "main", "main");
        git(repo, &["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main"]);
        git(repo, &["checkout", "-q", "-b", "feature", from]);
    }
    let web = f.root().join("web");
    write(&api, "a.txt", "feature\n");
    commit_all(&api, "feature");
    write(&web, "b.txt", "feature\n");
    commit_all(&web, "feature");
    (f, api, web)
}

#[test]
fn a_base_picked_over_a_folder_reaches_each_repository_that_has_it() {
    let (f, api, web) = forked();
    let mut app = app_on(&f.root(), None, Scope::Branch);
    app.open_base_picker();
    let picker = app.base_picker.as_ref().expect("the picker opens over a folder");
    let names: Vec<&str> = picker.rows.iter().map(herdr_reviewr::app::BaseChoice::name).collect();
    for name in ["dev", "main", "feature"] {
        assert!(names.contains(&name), "every repository's branches: {names:?}");
    }
    assert_eq!(names.iter().filter(|n| **n == "main").count(), 1, "one row per name");
    let at = picker.visible().iter().position(|r| r.name() == "dev").unwrap();
    app.base_picker_goto(at);
    app.base_picker_pick().unwrap();
    let pick = |repo: &Path| herdr_reviewr::git::read_base_pick(repo).unwrap();
    assert_eq!(pick(&api).as_deref(), Some("dev"));
    assert_eq!(pick(&web), None, "a repository without the branch falls back");
    let base = |name: &str| app.repo_bases[name].winner.as_ref().map(|w| w.name().to_string());
    assert_eq!(base("api").as_deref(), Some("dev"));
    assert_eq!(base("web").as_deref(), Some("main"));
    // The header names the base most changed repositories use, and counts the rest.
    let (shown, others) = app.folder_base().expect("a base");
    assert_eq!(others, 1, "{shown} and one other");
    let refs = app.watched_refs();
    assert!(
        refs.contains("api/refs/heads/dev") && refs.contains("web/refs/heads/main"),
        "{refs:?}"
    );
}

#[test]
fn a_folder_says_which_scopes_need_one_repository() {
    let f = Folder::new();
    f.repo("api", &[("a.txt", "one\n")]);
    let mut app = app_on(&f.root(), None, Scope::Uncommitted);
    app.set_scope(Scope::Commits).unwrap();
    assert_eq!(app.scope, Scope::Uncommitted, "the commit picker stays shut");
    assert!(app.status.contains("one repo"), "{}", app.status);
    app.set_scope(Scope::LastTurn).unwrap();
    assert!(app.turn_wait_message().contains("one repo"));
}

#[test]
fn an_ignored_folder_expanded_in_one_repository_lists_its_files() {
    let f = Folder::new();
    let api = f.repo("api", &[(".gitignore", "target/\n"), ("a.txt", "one\n")]);
    f.repo("web", &[("b.txt", "two\n")]);
    write(&api, "target/out.o", "x\n");
    let mut app = app_on(&f.root(), None, Scope::Uncommitted);
    app.set_tab(Tab::AllFiles).unwrap();
    assert!(listed(&app).contains(&"api/target".to_string()), "{:?}", listed(&app));
    assert!(!listed(&app).contains(&"api/target/out.o".to_string()), "unread until expanded");
    let expand = |app: &mut App, path: &str| {
        app.focus = herdr_reviewr::app::Focus::Files;
        app.file_cursor = app
            .file_rows
            .iter()
            .position(|r| r.dir_path() == Some(path))
            .unwrap_or_else(|| panic!("a row for {path}"));
        app.expand_dir();
    };
    expand(&mut app, "api");
    expand(&mut app, "api/target");
    let after = listed(&app);
    assert!(after.contains(&"api/target/out.o".to_string()), "{after:?}");
    assert!(after.contains(&"web/b.txt".to_string()), "the other repository stays listed");
}

#[test]
fn a_saved_default_base_rebuilds_the_branch_scope() {
    let f = Folder::new();
    let api = f.repo("api", &[("a.txt", "base\n")]);
    git(&api, &["checkout", "-q", "-b", "dev"]);
    write(&api, "dev-only.txt", "dev\n");
    commit_all(&api, "dev work");
    origin(&api, "main", "main");
    git(&api, &["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main"]);
    git(&api, &["checkout", "-q", "-b", "feature"]);
    write(&api, "a.txt", "feature\n");
    commit_all(&api, "feature");
    let mut app = app_on(&f.root(), None, Scope::Branch);
    assert_eq!(listed(&app), ["api/a.txt", "api/dev-only.txt"]);
    app.world_request = None;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.toml"), "default_base = \"dev\"\n").unwrap();
    app.set_plugin_config(herdr_reviewr::config::plugin_config_in(dir.path()).unwrap());
    assert!(app.world_request.take().is_some(), "the save asks for a rebuild");
    app.reconcile_world(world::build(&app.world_input()).unwrap());
    assert_eq!(listed(&app), ["api/a.txt"], "dev's own work is no change against dev");
}
