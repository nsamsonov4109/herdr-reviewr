//! A folder of repositories reviewed as one: each repository's changes under its name, each
//! diffed against its own ends and base, and a refresh re-reading only the repository it names.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use herdr_reviewr::app::{App, Tab};
use herdr_reviewr::model::{CommitPick, Picks, Scope};
use herdr_reviewr::repos::Repos;
use herdr_reviewr::turn::{Status, TurnHost, TurnReport, Wrote};
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
    git_env(dir, args, &[])
}

fn git_env(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> String {
    let out = Command::new("git")
        .envs(env.iter().copied())
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

/// Commit everything as `message`, `hour` hours into 2026.
fn commit_at(dir: &Path, message: &str, hour: u64) {
    let date = format!("@{} +0000", 1_767_225_600 + hour * 3600);
    git(dir, &["add", "-A"]);
    let env = [("GIT_AUTHOR_DATE", date.as_str()), ("GIT_COMMITTER_DATE", date.as_str())];
    git_env(dir, &["commit", "-q", "-m", message], &env);
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
        let job = WorldJob {
            generation: self.generation,
            input: input.clone(),
            reveal: false,
            refresh,
            probe: std::collections::BTreeMap::new(),
            rehome: std::collections::BTreeMap::new(),
        };
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
fn a_folder_waits_for_its_first_turn() {
    let f = Folder::new();
    f.repo("api", &[("a.txt", "one\n")]);
    let mut app = app_on(&f.root(), None, Scope::Uncommitted);
    app.set_scope(Scope::LastTurn).unwrap();
    assert_eq!(app.turn_wait_message(), "waiting for the first turn", "last turn takes a folder");
}

/// A folder whose `api` and `web` branch `feature` off `main`: `step one` in both, then `step
/// two` in `api`, an hour apart.
fn stepped() -> (Folder, PathBuf, PathBuf) {
    let f = Folder::new();
    let api = f.repo("api", &[("a.txt", "base\n")]);
    let web = f.repo("web", &[("b.txt", "base\n")]);
    for repo in [&api, &web] {
        origin(repo, "main", "main");
        git(repo, &["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main"]);
        git(repo, &["checkout", "-q", "-b", "feature"]);
    }
    write(&api, "a.txt", "one\n");
    commit_at(&api, "step one", 1);
    write(&web, "b.txt", "one\n");
    commit_at(&web, "step one", 2);
    write(&api, "a.txt", "two\n");
    commit_at(&api, "step two", 3);
    (f, api, web)
}

fn picker(app: &App) -> &herdr_reviewr::app::CommitPicker {
    app.commit_picker.as_ref().expect("the commit picker is open")
}

#[test]
fn the_commit_picker_lists_every_repositorys_commits_and_picks_a_step_in_each() {
    let (f, api, web) = stepped();
    let mut app = app_on(&f.root(), None, Scope::Uncommitted);
    app.set_scope(Scope::Commits).unwrap();
    let cp = picker(&app);
    let rows: Vec<(&str, &str)> =
        (0..cp.rows.len()).map(|i| (cp.repo_of(i), cp.rows[i].subject.as_str())).collect();
    assert_eq!(rows, [("api", "step two"), ("web", "step one"), ("api", "step one")]);
    assert_eq!(cp.title, "commits · 3 over their bases in 2 repos");
    assert_eq!(cp.opened(), BTreeSet::from([0]), "a step only api took");
    app.commit_picker_move(1);
    assert_eq!(picker(&app).opened(), BTreeSet::from([1, 2]), "a step both repositories took");
    app.commit_picker_pick().unwrap();
    assert_eq!(app.scope, Scope::Commits);
    let sha = |repo: &Path, rev: &str| git(repo, &["rev-parse", rev]);
    let picks = Picks::from([
        ("api".to_string(), CommitPick::single(&sha(&api, "HEAD~1"))),
        ("web".to_string(), CommitPick::single(&sha(&web, "HEAD"))),
    ]);
    assert_eq!(app.commit_picks, picks);
    assert_eq!(listed(&app), ["api/a.txt", "web/b.txt"]);
    let header = painted(&app).0.remove(0);
    assert!(header.contains("[commits] 2 commits in 2 repos · step one"), "{header}");
    let rows = open(&mut app, "api/a.txt");
    assert!(rows.contains(&"-base".to_string()) && rows.contains(&"+one".to_string()), "{rows:?}");
    let rows = open(&mut app, "web/b.txt");
    assert!(rows.contains(&"-base".to_string()) && rows.contains(&"+one".to_string()), "{rows:?}");
}

/// The frame's rows as text, and its footer: the last row with any.
fn painted(app: &App) -> (Vec<String>, String) {
    app.settle_answers();
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(140, 40)).unwrap();
    terminal.draw(|f| herdr_reviewr::ui::render(f, app)).unwrap();
    let buf = terminal.backend().buffer();
    let rows: Vec<String> = (0..buf.area.height)
        .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect())
        .collect();
    let footer = rows.iter().rev().find(|l| !l.trim().is_empty()).cloned().unwrap_or_default();
    (rows, footer)
}

#[test]
fn the_folder_picker_paints_each_rows_repository_and_what_enter_opens() {
    let (f, api, web) = stepped();
    let mut app = app_on(&f.root(), None, Scope::Uncommitted);
    app.open_commit_picker();
    app.commit_picker_move(1);
    let (rows, footer) = painted(&app);
    let row = |repo: &Path, rev: &str| {
        let sha = git(repo, &["rev-parse", "--short=7", rev]);
        rows.iter().find(|l| l.contains(&sha)).cloned().expect("a row per commit")
    };
    let (two, web_one, api_one) = (row(&api, "HEAD"), row(&web, "HEAD"), row(&api, "HEAD~1"));
    assert!(web_one.contains("web  ") && api_one.contains("api  "), "{web_one}\n{api_one}");
    assert!(web_one.contains('▎') && api_one.contains('▎'), "the step enter opens is barred");
    assert!(!two.contains('▎'), "{two}");
    assert!(footer.contains("enter open 2") && footer.contains("v select"), "{footer}");
    assert!(footer.contains("esc cancel"), "{footer}");
    app.commit_picker_anchor();
    let (_, footer) = painted(&app);
    assert!(footer.contains("v drop") && footer.contains("esc clear"), "{footer}");
    app.commit_picker_move(-1);
    let (_, footer) = painted(&app);
    assert!(footer.contains("v add"), "{footer}");
}

#[test]
fn in_branch_the_picker_lists_over_the_bases_the_view_shows() {
    let (f, _api, web) = stepped();
    let mut app = app_on(&f.root(), None, Scope::Branch);
    // web's base moves past its commit after the view landed: the picker reads the view's base.
    git(&web, &["update-ref", "refs/remotes/origin/main", "feature"]);
    app.open_commit_picker();
    assert_eq!(picker(&app).rows.len(), 3, "web's step one, over the base on screen");
    app.close_commit_picker();
    // Outside `branch` each repository resolves its base afresh.
    app.set_scope(Scope::Uncommitted).unwrap();
    app.open_commit_picker();
    assert_eq!(picker(&app).rows.len(), 2, "web's base now holds its commit");
}

#[test]
fn a_suggestion_across_repositories_opens_as_the_selection() {
    let (f, api, web) = stepped();
    let mut app = app_on(&f.root(), None, Scope::Uncommitted);
    let sha = |repo: &Path, rev: &str| git(repo, &["rev-parse", rev]);
    let picks = Picks::from([
        ("api".to_string(), CommitPick::single(&sha(&api, "HEAD"))),
        ("web".to_string(), CommitPick::single(&sha(&web, "HEAD"))),
    ]);
    app.receive_suggestion(picks.clone(), "both halves", Some("claude".into()));
    assert_eq!(app.status, "claude suggests commits in 2 repos · G opens them");
    app.open_commit_picker();
    assert_eq!(picker(&app).checked, BTreeSet::from([0, 1]), "api's step two, web's step one");
    assert_eq!(picker(&app).title, "claude suggests: both halves");
    app.commit_picker_pick().unwrap();
    assert_eq!(app.commit_picks, picks);
    assert_eq!(listed(&app), ["api/a.txt", "web/b.txt"]);
}

#[test]
fn moving_off_the_suggestion_the_picker_opened_on_picks_the_highlighted_commit() {
    let (f, api, web) = stepped();
    let mut app = app_on(&f.root(), None, Scope::Uncommitted);
    let sha = |repo: &Path, rev: &str| git(repo, &["rev-parse", rev]);
    let suggested = Picks::from([("web".to_string(), CommitPick::single(&sha(&web, "HEAD")))]);
    app.receive_suggestion(suggested, "web first", Some("claude".into()));
    app.open_commit_picker();
    assert_eq!(picker(&app).checked, BTreeSet::from([1]), "it opens on web's step one");
    // The reviewer moves to api's step two: `enter` takes the highlight, not the suggestion.
    app.commit_picker_move(-1);
    assert!(picker(&app).checked.is_empty(), "the selection it opened on is dropped");
    app.commit_picker_pick().unwrap();
    let picked = Picks::from([("api".to_string(), CommitPick::single(&sha(&api, "HEAD")))]);
    assert_eq!(app.commit_picks, picked);
    assert!(app.suggestions.is_empty(), "and the pick answers the suggestion");
}

#[test]
fn a_folder_selection_adds_and_drops_rows_and_reopens_as_picked() {
    let (f, api, _web) = stepped();
    let mut app = app_on(&f.root(), None, Scope::Uncommitted);
    app.open_commit_picker();
    app.commit_picker_move(1);
    // `v` selects the highlight's step; then each `v` adds or drops one row.
    app.commit_picker_anchor();
    assert_eq!(picker(&app).checked, BTreeSet::from([1, 2]));
    app.commit_picker_move(-1);
    app.commit_picker_anchor();
    app.commit_picker_move(1);
    app.commit_picker_anchor();
    assert_eq!(picker(&app).checked, BTreeSet::from([0, 2]), "web's step dropped");
    app.commit_picker_pick().unwrap();
    let sha = |rev: &str| git(&api, &["rev-parse", rev]);
    let run = CommitPick { oldest: sha("HEAD~1"), newest: sha("HEAD") };
    let label = format!("{} to {} (2) · api", &run.oldest[..7], &run.newest[..7]);
    assert_eq!(app.commit_picks, Picks::from([("api".to_string(), run)]));
    assert_eq!(listed(&app), ["api/a.txt"]);
    let header = painted(&app).0.remove(0);
    assert!(header.contains(&label), "one repository's run names it: {header}");
    // No highlight's step equals the pick, so it reopens as the selection, which esc clears.
    app.open_commit_picker();
    assert_eq!((picker(&app).cursor, picker(&app).checked.clone()), (0, BTreeSet::from([0, 2])));
    app.commit_picker_escape();
    assert!(picker(&app).checked.is_empty());
    assert_eq!(picker(&app).opened(), BTreeSet::from([0]));
    app.commit_picker_escape();
    assert!(app.commit_picker.is_none(), "the second esc closes");
    assert_eq!(app.commit_picks.len(), 1, "and keeps the pick");
}

/// One status of an agent at the folder's root, folded in as the herdr connection does.
fn sample(host: &mut TurnHost, root: &Path, status: Status) -> TurnReport {
    let cwd = root.to_string_lossy().into_owned();
    let members = host.members(&[("w1:p1", Some(cwd.as_str()))]).expect("resolved");
    assert_eq!(members, ["w1:p1"], "an agent at the folder's root works in it");
    host.observe_statuses(Some(&[status]), Instant::now())
}

#[test]
fn a_last_turn_lists_what_the_turn_changed_in_each_repository() {
    let f = Folder::new();
    let api = f.repo("api", &[("a.txt", "one\n")]);
    let web = f.repo("web", &[("b.txt", "two\n")]);
    // Uncommitted before the turn, so no part of it.
    write(&api, "a.txt", "one, before the turn\n");
    let root = std::fs::canonicalize(f.root()).unwrap();
    let repos = Repos::discover(&root);
    let mut host = TurnHost::open(root.clone(), repos.clone());
    assert_eq!(host.members(&[("w9:p1", Some("/elsewhere"))]), Some(Vec::new()), "outside");
    sample(&mut host, &root, Status::Idle);
    sample(&mut host, &root, Status::Working);
    write(&web, "b.txt", "two, in the turn\n");
    let later = Instant::now() + Duration::from_secs(1);
    let paths = Some(["web/b.txt".to_string()].into());
    assert!(host.on_worktree_change(&Wrote { first: later, last: later, paths }), "promoted");
    let trees = host.baseline().expect("a baseline").clone();
    assert_eq!(trees.keys().collect::<Vec<_>>(), ["api", "web"], "every repository's tree");
    let persisted = herdr_reviewr::git::read_folder_baseline_ref(&web);
    assert_eq!(persisted.as_deref(), trees.get("web").map(String::as_str), "kept in each");
    assert_eq!(herdr_reviewr::git::read_baseline_ref(&web), None, "a pane on web keeps its own");

    let mut app = app_on(&root, None, Scope::Uncommitted);
    app.sync_turn(host.status());
    app.set_scope(Scope::LastTurn).unwrap();
    assert_eq!(listed(&app), ["web/b.txt"], "the turn's change, not the one before it");
    // A reopened pane resumes the turn from its repositories.
    assert_eq!(TurnHost::open(root, repos).baseline(), Some(&trees));
}

#[test]
fn a_write_beside_the_repositories_promotes_no_turn() {
    let f = Folder::new();
    f.repo("api", &[("a.txt", "one\n")]);
    let root = std::fs::canonicalize(f.root()).unwrap();
    let mut host = TurnHost::open(root.clone(), Repos::discover(&root));
    sample(&mut host, &root, Status::Idle);
    sample(&mut host, &root, Status::Working);
    std::fs::write(root.join("notes.txt"), "beside\n").unwrap();
    let later = Instant::now() + Duration::from_secs(1);
    let paths = Some(["notes.txt".to_string()].into());
    assert!(!host.on_worktree_change(&Wrote { first: later, last: later, paths }));
    assert_eq!(host.baseline(), None, "no repository changed");
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

#[test]
fn a_round_since_the_send_lists_only_what_changed_after_it_in_each_repository() {
    let f = Folder::new();
    let api = f.repo("api", &[("a.txt", "one\n")]);
    let web = f.repo("web", &[("b.txt", "two\n")]);
    // Before the send: an edit the round must not show.
    write(&api, "a.txt", "ONE\n");
    let mut app = app_on(&f.root(), None, Scope::Uncommitted);
    let (baselines, failed) = world::snapshot_all(&f.root(), app.repos());
    assert!(failed.is_empty(), "{failed:?}");
    assert_eq!(baselines.keys().collect::<Vec<_>>(), ["api", "web"]);
    app.land_send_baselines(baselines, &failed);
    // After it: the agent edits both repositories, one file new.
    write(&api, "a.txt", "ONE more\n");
    write(&web, "b.txt", "TWO\n");
    write(&web, "c.txt", "new\n");
    app.toggle_since_send().unwrap();
    assert_eq!(app.scope, Scope::SinceSend);
    assert_eq!(listed(&app), ["api/a.txt", "web/b.txt", "web/c.txt"]);
    let rows = open(&mut app, "api/a.txt");
    assert!(
        rows.contains(&"-ONE".to_string()) && rows.contains(&"+ONE more".to_string()),
        "{rows:?}"
    );
    app.toggle_since_send().unwrap();
    assert_eq!(app.scope, Scope::Uncommitted, "the key goes back where it came from");
}
