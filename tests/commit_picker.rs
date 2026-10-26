//! The commit picker and agent suggestions at their edges, in one repository and in a folder.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{Repo, app_on};
use herdr_reviewr::app::{App, Mode};
use herdr_reviewr::keymap::Keymap;
use herdr_reviewr::model::{CommitPick, Picks, Scope};
use herdr_reviewr::repos::Repos;
use herdr_reviewr::{handle_key, handle_mouse};
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;

const AREA: Rect = Rect { x: 0, y: 0, width: 120, height: 40 };

fn press(app: &mut App, keymap: &Keymap, code: KeyCode) {
    handle_key(app, KeyEvent::from(code), AREA, keymap).unwrap();
}

/// Paint one frame, as the event loop does between inputs.
fn paint(app: &App) {
    let backend = ratatui::backend::TestBackend::new(AREA.width, AREA.height);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal.draw(|f| herdr_reviewr::ui::render(f, app)).unwrap();
}

/// One mouse event at a cell, after the frame the user saw.
fn mouse_at(app: &mut App, kind: MouseEventKind, column: u16, row: u16) {
    paint(app);
    let heights = herdr_reviewr::ui::diff_row_heights(app, AREA);
    let event = MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE };
    let clipboard = herdr_reviewr::export::Clipboard;
    handle_mouse(app, event, AREA, &heights, &Keymap::default(), &clipboard).unwrap();
}

/// Every screen cell over picker row `i`.
fn picker_cells(app: &App, i: usize) -> Vec<(u16, u16)> {
    (0..AREA.height)
        .flat_map(|row| (0..AREA.width).map(move |col| (col, row)))
        .filter(|&(col, row)| {
            herdr_reviewr::ui::hit_commit_picker_row(AREA, app, col, row) == Some(i)
        })
        .collect()
}

/// Four commits each adding a file, an edit to `root.rs`; the shas root first.
fn commits_repo() -> (Repo, Vec<String>) {
    let r = Repo::init();
    r.write("root.rs", "r\n");
    r.commit_all("root");
    r.write("one.rs", "1\n");
    r.commit_all("one");
    r.write("two.rs", "2\n");
    r.commit_all("two");
    r.write("three.rs", "3\n");
    r.commit_all("three");
    r.write("root.rs", "dirty\n");
    let shas: Vec<String> =
        r.git(&["rev-list", "--reverse", "HEAD"]).lines().map(str::to_string).collect();
    (r, shas)
}

fn picker(app: &App) -> &herdr_reviewr::app::CommitPicker {
    app.commit_picker.as_ref().expect("the commit picker is open")
}

fn one(sha: &str) -> Picks {
    Picks::from([(String::new(), CommitPick::single(sha))])
}

// --- folder fixtures ----------------------------------------------------------

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

fn origin(dir: &Path, name: &str, rev: &str) {
    let oid = git(dir, &["rev-parse", rev]);
    git(dir, &["update-ref", &format!("refs/remotes/origin/{name}"), &oid]);
}

fn folder_app(root: &Path) -> App {
    let mut app = App::new(root.to_path_buf(), Scope::Uncommitted, None);
    app.set_repos(Repos::discover(root));
    app.reload().unwrap();
    app
}

/// `api` and `web` branch `feature` off `main`: `step one` in both, then `step two` in `api`.
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

/// The picker the suggestion opened claims the agent's suggestion only while it selects all of it.
fn assert_claims_only_what_it_selects(app: &App, suggested: &Picks) {
    let cp = picker(app);
    assert!(
        !cp.suggested || cp.picks() == *suggested,
        "titled {:?}, but enter takes {:?}, not the suggestion {suggested:?} (status {:?})",
        cp.title,
        cp.picks(),
        app.status
    );
}

#[test]
fn a_double_click_on_the_highlighted_row_picks_and_goes_no_further() {
    let r = Repo::init();
    r.write("a.rs", &(1..=60).map(|i| format!("line {i}\n")).collect::<Vec<_>>().concat());
    r.commit_all("root");
    r.write("a.rs", &(1..=60).map(|i| format!("line {i} changed\n")).collect::<Vec<_>>().concat());
    r.commit_all(
        "Rework the parser so that every line of the input is read twice over, once for the \
         tokens and once for the spans, and say why in the commit body",
    );
    let mut app = app_on(&r);
    // `G` opens with the newest commit highlighted; the reviewer double-clicks it.
    app.open_commit_picker();
    assert_eq!(picker(&app).cursor, 0);
    let (col, row) = picker_cells(&app, 0)[0];
    for kind in [MouseEventKind::Down(MouseButton::Left), MouseEventKind::Up(MouseButton::Left)] {
        mouse_at(&mut app, kind, col, row);
    }
    assert_eq!(
        (app.mode.clone(), app.scope),
        (Mode::Normal, Scope::Commits),
        "the first click picks"
    );
    for kind in [MouseEventKind::Down(MouseButton::Left), MouseEventKind::Up(MouseButton::Left)] {
        mouse_at(&mut app, kind, col, row);
    }
    assert_eq!(app.mode, Mode::Normal, "the double-click's second click opened a comment box");
}

#[test]
fn a_pick_answers_only_a_suggestion_the_picker_showed() {
    let (r, shas) = commits_repo();
    let mut app = app_on(&r);
    let keymap = Keymap::default();
    press(&mut app, &keymap, KeyCode::Char('G'));
    press(&mut app, &keymap, KeyCode::Char('j'));
    // The agent's suggestion lands while the reviewer is choosing `two`.
    app.receive_suggestion(one(&shas[1]), "look at one", Some("claude".into()));
    assert_eq!(picker(&app).title, "commits · last 50", "the open picker never shows it");
    press(&mut app, &keymap, KeyCode::Char('G'));
    assert_eq!(app.mode, Mode::CommitPick, "and `G` is inert inside the picker");
    press(&mut app, &keymap, KeyCode::Enter);
    assert_eq!(app.commit_pick(), Some(&CommitPick::single(&shas[2])));
    assert_eq!(app.suggestions.len(), 1, "the unseen suggestion was dropped by this pick");
}

#[test]
fn a_folder_suggestion_one_repository_lost_its_commit_of_never_opens_as_a_part() {
    let (f, api, web) = stepped();
    let gone = git(&web, &["rev-parse", "HEAD"]);
    // The agent amends web's step after suggesting it.
    write(&web, "b.txt", "one, amended\n");
    git(&web, &["add", "-A"]);
    git(&web, &["commit", "-q", "--amend", "-m", "step one"]);
    let mut app = folder_app(&f.root());
    let suggested = Picks::from([
        ("api".to_string(), CommitPick::single(&git(&api, &["rev-parse", "HEAD~1"]))),
        ("web".to_string(), CommitPick::single(&gone)),
    ]);
    app.receive_suggestion(suggested.clone(), "step one in both", Some("claude".into()));
    app.open_commit_picker();
    assert_claims_only_what_it_selects(&app, &suggested);
}

#[test]
fn a_folder_suggested_run_across_a_merge_never_opens_as_its_newest_commit_alone() {
    let (f, api, _web) = stepped();
    // api: a side branch merged back with --no-ff, then one more commit.
    git(&api, &["checkout", "-q", "-b", "side"]);
    write(&api, "side.txt", "s1\n");
    commit_at(&api, "side one", 4);
    write(&api, "side.txt", "s2\n");
    commit_at(&api, "side two", 5);
    git(&api, &["checkout", "-q", "feature"]);
    let date = format!("@{} +0000", 1_767_225_600 + 6 * 3600);
    let env = [("GIT_AUTHOR_DATE", date.as_str()), ("GIT_COMMITTER_DATE", date.as_str())];
    git_env(&api, &["merge", "-q", "--no-ff", "-m", "merge side", "side"], &env);
    write(&api, "a.txt", "three\n");
    commit_at(&api, "step three", 7);
    // What `--suggest <side one>,<step three>` resolves to: one run, oldest to newest.
    let run = CommitPick {
        oldest: git(&api, &["rev-parse", "side~1"]),
        newest: git(&api, &["rev-parse", "HEAD"]),
    };
    let suggested = Picks::from([("api".to_string(), run)]);
    let mut app = folder_app(&f.root());
    app.receive_suggestion(suggested.clone(), "the side work and what followed", None);
    app.open_commit_picker();
    assert_claims_only_what_it_selects(&app, &suggested);
}

#[test]
fn a_folder_suggested_run_reaching_below_the_base_never_opens_as_its_newest_commit_alone() {
    let (f, api, _web) = stepped();
    // From api's base commit through step two: the base commit is not over the base.
    let run = CommitPick {
        oldest: git(&api, &["rev-parse", "HEAD~2"]),
        newest: git(&api, &["rev-parse", "HEAD"]),
    };
    let suggested = Picks::from([("api".to_string(), run)]);
    let mut app = folder_app(&f.root());
    app.receive_suggestion(suggested.clone(), "the whole api run", None);
    app.open_commit_picker();
    assert_claims_only_what_it_selects(&app, &suggested);
}

#[test]
fn a_relisted_picker_lets_go_of_a_suggestion_whose_commits_were_rewritten() {
    let (r, shas) = commits_repo();
    let mut app = app_on(&r);
    let run = CommitPick { oldest: shas[1].clone(), newest: shas[2].clone() };
    let suggested = Picks::from([(String::new(), run.clone())]);
    app.receive_suggestion(suggested.clone(), "one and two", Some("claude".into()));
    app.open_commit_picker();
    assert_eq!(picker(&app).picked(), Some(run));
    // The agent rewrites the suggested commits while the picker is up.
    r.git(&["reset", "-q", "--hard", &shas[0]]);
    for (file, subject) in [("one.rs", "one again"), ("two.rs", "two again"), ("three.rs", "three")]
    {
        r.write(file, &format!("{subject}\n"));
        r.commit_all(subject);
    }
    common::land_world(&mut app);
    // Nearest survivor is `root`: enter would pick it under the agent's reason, and answer it.
    assert_claims_only_what_it_selects(&app, &suggested);
}

#[test]
fn a_relist_keeps_the_highlight_on_the_pick_as_it_becomes_the_pick_row() {
    let (r, shas) = commits_repo();
    let mut app = app_on(&r);
    let keymap = Keymap::default();
    press(&mut app, &keymap, KeyCode::Char('G'));
    press(&mut app, &keymap, KeyCode::Enter);
    // `G` opens on what is picked now: `three`.
    press(&mut app, &keymap, KeyCode::Char('G'));
    assert_eq!(picker(&app).picked(), Some(CommitPick::single(&shas[3])));
    // The agent drops `three` under the open picker; `three` stays on screen as the pick row.
    r.git(&["reset", "-q", "--hard", &shas[2]]);
    common::land_world(&mut app);
    assert_eq!(picker(&app).pick_row, Some(CommitPick::single(&shas[3])));
    assert_eq!(
        picker(&app).picked(),
        Some(CommitPick::single(&shas[3])),
        "the reviewer never moved, yet enter now picks `two`"
    );
}

#[test]
fn one_unreadable_repository_keeps_the_folder_picker_open_for_the_rest() {
    let (f, _api, _web) = stepped();
    // A linked worktree whose repository is gone: its `.git` file names nothing.
    let stale = f.root().join("stale");
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::write(stale.join(".git"), "gitdir: /no/such/repo/.git/worktrees/stale\n").unwrap();
    let mut app = folder_app(&f.root());
    app.open_commit_picker();
    assert_eq!(app.mode, Mode::CommitPick, "api's and web's commits still list: {:?}", app.status);
}

#[test]
fn a_folder_of_a_repository_and_its_linked_worktree_lists_the_worktrees_commit() {
    let f = Folder::new();
    let main = f.repo("main", &[("a.txt", "base\n")]);
    let feature = f.root().join("feature");
    git(&main, &["worktree", "add", "-q", "-b", "feature", feature.to_str().unwrap()]);
    write(&feature, "a.txt", "feature\n");
    commit_all(&feature, "feature work");
    let sha = git(&feature, &["rev-parse", "HEAD"]);
    let mut app = folder_app(&f.root());
    app.open_commit_picker();
    let cp = picker(&app);
    let rows: Vec<(&str, &str)> =
        (0..cp.rows.len()).map(|i| (cp.repo_of(i), cp.rows[i].sha.as_str())).collect();
    assert_eq!(rows, [("feature", sha.as_str())], "the pane lists it; only `--suggest` refuses it");
}
