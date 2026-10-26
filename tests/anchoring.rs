//! Where comments sit as the code under them changes: edits, scopes, rewrites and links.

mod common;

use common::{Repo, app_on};
use herdr_reviewr::app::{App, Focus};
use herdr_reviewr::model::{Scope, Side};

/// The visible file-list row index for `path`.
fn file_row(app: &App, path: &str) -> usize {
    app.file_rows
        .iter()
        .position(|r| r.file_index().is_some_and(|i| app.entries[i].path == path))
        .unwrap_or_else(|| panic!("a file row for {path}"))
}

/// Open `path` in the current tab.
fn open(app: &mut App, path: &str) {
    let row = file_row(app, path);
    app.select_file(row).unwrap();
}

/// Comment `text` on the open view's row reading `marker + line`.
fn comment_on_row(app: &mut App, row: &str, text: &str) {
    app.focus = Focus::Diff;
    app.diff_cursor = app
        .visible
        .iter()
        .position(|v| v.row().is_some_and(|r| format!("{}{}", r.marker(), r.text()) == row))
        .unwrap_or_else(|| panic!("a row reading {row:?}"));
    app.start_comment();
    for ch in text.chars() {
        app.input_push(ch);
    }
    app.submit_comment();
}

/// Where comment 0 sits: side, start, and whether its card says `code changed`.
fn place(app: &App) -> (Side, u32, bool) {
    let p = app.card_place(0).expect("the open view shows comment 0");
    (p.side, p.start, p.changed)
}

const BASE: &str = "fn alpha() -> u32 { 1 }\nfn bravo() -> u32 { 2 }\nfn charlie() -> u32 { 3 }\n\
                    fn delta() -> u32 { 4 }\n";

#[test]
fn a_stash_round_trip_leaves_a_comment_on_its_line() {
    let r = Repo::init();
    r.write("lib.rs", BASE);
    r.commit_all("base");
    // The agent adds a line between bravo and charlie; the reviewer comments on it.
    let edited = BASE.replace("fn charlie", "fn inserted_helper() -> u32 { 42 }\nfn charlie");
    r.write("lib.rs", &edited);
    let mut app = app_on(&r);
    open(&mut app, "lib.rs");
    comment_on_row(&mut app, "+fn inserted_helper() -> u32 { 42 }", "why 42?");
    assert_eq!(place(&app), (Side::New, 3, false));
    // The agent stashes to check something on a clean tree, then pops: the file reads as before.
    r.git(&["stash", "-q"]);
    common::land_world(&mut app);
    r.git(&["stash", "pop", "-q"]);
    common::land_world(&mut app);
    assert_eq!(std::fs::read_to_string(r.path().join("lib.rs")).unwrap(), edited);
    open(&mut app, "lib.rs");
    assert_eq!(place(&app), (Side::New, 3, false), "the file is byte for byte what it was");
    let location = app.store.get(0).unwrap().location();
    assert_eq!(location, "lib.rs:3", "a send names the line the comment was made on");
}

#[test]
fn a_file_read_while_emptied_and_then_rewritten_keeps_its_comment() {
    let r = Repo::init();
    r.write("lib.rs", BASE);
    r.commit_all("base");
    let edited = BASE.replace("{ 2 }", "{ 22 }");
    r.write("lib.rs", &edited);
    let mut app = app_on(&r);
    open(&mut app, "lib.rs");
    comment_on_row(&mut app, "+fn bravo() -> u32 { 22 }", "why 22?");
    // A refresh reads the file between a truncate and the write that follows it.
    r.write("lib.rs", "");
    common::land_world(&mut app);
    r.write("lib.rs", &edited);
    common::land_world(&mut app);
    open(&mut app, "lib.rs");
    assert_eq!(place(&app), (Side::New, 2, false), "the file reads as it did");
}

#[test]
fn a_scope_round_trip_leaves_a_removed_line_comment_on_its_line() {
    let r = Repo::init();
    r.write("lib.rs", BASE);
    r.commit_all("base");
    r.set_origin_default("main", "HEAD");
    r.git(&["checkout", "-q", "-b", "feature"]);
    // The branch added a helper; the worktree takes it out again and edits delta.
    let added = BASE.replace("fn charlie", "fn inserted_helper() -> u32 { 42 }\nfn charlie");
    r.write("lib.rs", &added);
    r.commit_all("add helper");
    r.write("lib.rs", &BASE.replace("{ 4 }", "{ 40 }"));
    let mut app = app_on(&r);
    open(&mut app, "lib.rs");
    comment_on_row(&mut app, "-fn inserted_helper() -> u32 { 42 }", "why drop it?");
    assert_eq!(place(&app), (Side::Old, 3, false));
    // The reviewer glances at the branch scope, then comes back.
    app.set_scope(Scope::Branch).unwrap();
    open(&mut app, "lib.rs");
    app.set_scope(Scope::Uncommitted).unwrap();
    open(&mut app, "lib.rs");
    assert_eq!(place(&app), (Side::Old, 3, false), "nothing in the repository changed");
}

/// `a.rs` and `b.rs` each gain a line the reviewer comments on; then the agent deletes `b.rs`,
/// whose commented line was never committed, so no side of its diff holds it.
fn deleted_file_review() -> (Repo, App) {
    let r = Repo::init();
    r.write("a.rs", "fn a() {}\n");
    r.write("b.rs", "fn b() {}\n");
    r.commit_all("base");
    r.write("a.rs", "fn a() {}\nfn a_helper() -> u32 { 1 }\n");
    r.write("b.rs", "fn b() {}\nfn b_helper() -> u32 { 2 }\n");
    let mut app = app_on(&r);
    open(&mut app, "a.rs");
    comment_on_row(&mut app, "+fn a_helper() -> u32 { 1 }", "on a");
    open(&mut app, "b.rs");
    comment_on_row(&mut app, "+fn b_helper() -> u32 { 2 }", "on b");
    r.remove("b.rs");
    common::land_world(&mut app);
    open(&mut app, "b.rs");
    assert!(app.card_rows().is_empty(), "opening b.rs shows no comment");
    (r, app)
}

#[test]
fn next_comment_never_crosses_into_a_file_that_shows_none() {
    let (_r, mut app) = deleted_file_review();
    open(&mut app, "a.rs");
    let (row, ci) = app.card_rows()[0];
    assert_eq!(app.store.get(ci).unwrap().text, "on a");
    app.focus = Focus::Diff;
    app.diff_cursor = row;
    app.target_comment_card(ci);
    // `n` past a.rs's only comment: no other file shows one, so it comes around to a.rs's.
    app.jump_comment(1);
    assert_eq!(app.diff_path.as_deref(), Some("a.rs"), "n opened a file with no comment in it");
}

#[test]
fn a_new_answer_on_a_comment_no_view_shows_marks_no_file() {
    let (_r, mut app) = deleted_file_review();
    open(&mut app, "a.rs");
    let target = common_fake_target();
    assert!(app.export(&target));
    app.receive_reply(2, "removed the file".into(), None);
    assert!(!app.has_new_answer("b.rs"), "opening b.rs shows no thread, so no dot promises one");
}

#[test]
fn next_answer_on_a_comment_no_view_shows_opens_the_list() {
    let (_r, mut app) = deleted_file_review();
    open(&mut app, "a.rs");
    assert!(app.export(&common_fake_target()));
    app.receive_reply(2, "removed the file".into(), None);
    app.jump_answer();
    let shown = app.card_rows().iter().any(|&(_, ci)| ci == 1);
    assert_eq!(
        (app.mode.clone(), app.diff_path.clone(), shown, app.store.unread()),
        (herdr_reviewr::app::Mode::List, Some("a.rs".to_string()), false, 0),
        "A must open the comments list on #2, not a file that hides it"
    );
}

/// Pick commit `sha` alone through the picker.
fn pick(app: &mut App, sha: &str) {
    app.open_commit_picker();
    let at = app.commit_picker.as_ref().unwrap().index_of(sha).expect("the commit is listed");
    app.commit_picker_goto(at);
    app.commit_picker_pick().unwrap();
}

/// Run one refresh of `refresh` through the real world worker and land it, as the loop does.
fn worker_refresh(app: &mut App, refresh: herdr_reviewr::world::Refresh) {
    use herdr_reviewr::world::{self, WorldJob};
    let (job_tx, job_rx) = std::sync::mpsc::channel();
    let (res_tx, res_rx) = herdr_reviewr::wake::channel(&herdr_reviewr::wake::Waker::detached());
    let job = WorldJob {
        generation: 1,
        input: app.world_input(),
        reveal: false,
        refresh,
        probe: app.probe_paths(),
        rehome: app.comment_picks(),
    };
    job_tx.send(job).unwrap();
    drop(job_tx);
    let worker = world::spawn(app.repo.clone(), job_rx, res_tx);
    let done = res_rx.recv().expect("a completion");
    worker.join().unwrap();
    let live = herdr_reviewr::LiveJob { generation: 1, ..herdr_reviewr::LiveJob::default() };
    herdr_reviewr::land_world_completion(app, done, &live);
}

/// A send target that keeps what it was handed.
struct Capture(std::cell::RefCell<String>);

impl herdr_reviewr::export::ExportTarget for Capture {
    fn label(&self) -> &'static str {
        "capture"
    }
    fn reader(&self) -> Option<&str> {
        None
    }
    fn success_message(&self, what: &str) -> String {
        format!("exported {what}")
    }
    fn failure_message(&self, _error: &anyhow::Error, _copy: &str) -> String {
        String::new()
    }
    fn export(&self, text: &str) -> anyhow::Result<()> {
        *self.0.borrow_mut() = text.to_string();
        Ok(())
    }
}

#[test]
fn a_send_from_the_commits_scope_names_where_a_worktree_comment_is_now() {
    let r = Repo::init();
    r.write("lib.rs", BASE);
    r.commit_all("base");
    let head = r.git(&["rev-parse", "HEAD"]).trim().to_string();
    let edited = BASE.replace("fn charlie", "fn inserted_helper() -> u32 { 42 }\nfn charlie");
    r.write("lib.rs", &edited);
    let mut app = app_on(&r);
    open(&mut app, "lib.rs");
    comment_on_row(&mut app, "+fn inserted_helper() -> u32 { 42 }", "why 42?");
    // The reviewer goes to review a commit; meanwhile the agent adds two lines on top.
    pick(&mut app, &head);
    assert_eq!(app.scope, Scope::Commits);
    r.write("lib.rs", &format!("// one\n// two\n{edited}"));
    worker_refresh(&mut app, herdr_reviewr::world::Refresh::paths(["lib.rs".to_string()]));
    let sent = Capture(std::cell::RefCell::new(String::new()));
    assert!(app.export(&sent));
    let text = sent.0.borrow().clone();
    assert!(text.contains("lib.rs:5"), "the commented line is line 5 now:\n{text}");
}

/// `a.rs` and `lib.rs` changed in the worktree, the reviewer's comment on lib.rs's new line, and
/// `a.rs` open: lib.rs's comment is followed only by the worker's read-back.
fn two_file_review() -> (Repo, App, String, String) {
    let r = Repo::init();
    r.write("a.rs", "fn a() {}\n");
    r.write("lib.rs", BASE);
    r.commit_all("base");
    let head = r.git(&["rev-parse", "HEAD"]).trim().to_string();
    r.write("a.rs", "fn a() { todo!() }\n");
    let edited = BASE.replace("fn charlie", "fn inserted_helper() -> u32 { 42 }\nfn charlie");
    r.write("lib.rs", &edited);
    let mut app = app_on(&r);
    open(&mut app, "lib.rs");
    comment_on_row(&mut app, "+fn inserted_helper() -> u32 { 42 }", "why 42?");
    open(&mut app, "a.rs");
    (r, app, head, edited)
}

#[test]
fn on_uncommitted_the_worker_carries_a_worktree_comment() {
    let (r, mut app, _head, edited) = two_file_review();
    r.write("lib.rs", &format!("// one\n// two\n{edited}"));
    worker_refresh(&mut app, herdr_reviewr::world::Refresh::paths(["lib.rs".to_string()]));
    assert_eq!(app.store.get(0).unwrap().location(), "lib.rs:5");
}

#[test]
fn back_on_uncommitted_a_worktree_comment_still_names_its_old_line() {
    let (r, mut app, head, edited) = two_file_review();
    pick(&mut app, &head);
    r.write("lib.rs", &format!("// one\n// two\n{edited}"));
    worker_refresh(&mut app, herdr_reviewr::world::Refresh::paths(["lib.rs".to_string()]));
    app.set_scope(Scope::Uncommitted).unwrap();
    assert_eq!(app.diff_path.as_deref(), Some("a.rs"));
    assert_eq!(app.store.get(0).unwrap().location(), "lib.rs:5", "what a send would name");
}

/// The full sha `rev` names.
fn sha(r: &Repo, rev: &str) -> String {
    r.git(&["rev-parse", rev]).trim().to_string()
}

#[test]
fn a_branch_switch_and_back_leaves_a_commit_comment_on_its_commit() {
    let r = Repo::init();
    r.write("lib.rs", BASE);
    r.commit_all("base");
    // Another branch the agent also works on carries an unrelated "Fix lint".
    r.git(&["checkout", "-q", "-b", "side"]);
    r.write("other.rs", "fn other() {}\n");
    r.commit_all("Fix lint");
    let unrelated = sha(&r, "HEAD");
    r.git(&["checkout", "-q", "main"]);
    // The branch under review: "Fix lint" twice, the reviewer on the first.
    r.write("lib.rs", &BASE.replace("{ 2 }", "{ 22 }"));
    r.commit_all("Fix lint");
    let reviewed = sha(&r, "HEAD");
    r.write("third.rs", "fn third() {}\n");
    r.commit_all("Fix lint");
    let mut app = app_on(&r);
    pick(&mut app, &reviewed);
    open(&mut app, "lib.rs");
    comment_on_row(&mut app, "+fn bravo() -> u32 { 22 }", "why 22?");
    // The agent looks at its other branch, then comes back: no commit was rewritten.
    r.git(&["checkout", "-q", "side"]);
    common::land_world(&mut app);
    let moved = app.store.get(0).unwrap().location();
    r.git(&["checkout", "-q", "main"]);
    common::land_world(&mut app);
    let location = app.store.get(0).unwrap().location();
    assert_eq!(
        location,
        format!("lib.rs:2 (in commit {})", &reviewed[..7]),
        "on side it read {moved}; {} is side's unrelated commit",
        &unrelated[..7]
    );
}

#[test]
fn an_amended_first_commit_is_what_its_comment_follows() {
    use herdr_reviewr::git::rewritten;
    use herdr_reviewr::model::CommitPick;
    // A new project: the agent's first commit is the root, which it then amends.
    let r = Repo::init();
    r.write("lib.rs", BASE);
    r.commit_all("Start the parser");
    let root = sha(&r, "HEAD");
    r.write("lib.rs", &BASE.replace("{ 2 }", "{ 22 }"));
    r.git(&["commit", "-q", "-a", "--amend", "--no-edit"]);
    let amended = sha(&r, "HEAD");
    assert_eq!(
        rewritten(r.path(), &CommitPick::single(&root)),
        Some(CommitPick::single(&amended)),
        "an amend of the root is a rewrite like any other"
    );
}

#[test]
fn a_rebase_that_edits_the_root_carries_every_later_commits_comment() {
    use herdr_reviewr::git::rewritten;
    use herdr_reviewr::model::CommitPick;
    let r = Repo::init();
    r.write("lib.rs", BASE);
    r.commit_all("Start the parser");
    let root = sha(&r, "HEAD");
    r.write("parse.rs", "fn parse() {}\n");
    r.commit_all("Add parse");
    let later = sha(&r, "HEAD");
    // `git rebase -i --root` editing the root: a new root, `Add parse` replayed on it.
    r.git(&["checkout", "-q", "--orphan", "rewrite"]);
    r.git(&["rm", "-q", "-r", "--cached", "."]);
    r.remove("parse.rs");
    r.write("lib.rs", &BASE.replace("{ 2 }", "{ 22 }"));
    r.commit_all("Start the parser");
    r.git(&["cherry-pick", &later]);
    let replayed = sha(&r, "HEAD");
    r.git(&["branch", "-q", "-D", "main"]);
    r.git(&["branch", "-q", "-m", "main"]);
    assert_ne!(sha(&r, "HEAD~1"), root);
    assert_eq!(
        rewritten(r.path(), &CommitPick::single(&later)),
        Some(CommitPick::single(&replayed)),
        "`Add parse` was replayed unchanged"
    );
}

#[test]
fn a_look_at_since_send_leaves_a_removed_line_thread_on_its_line() {
    let r = Repo::init();
    r.write("lib.rs", BASE);
    r.commit_all("base");
    r.set_origin_default("main", "HEAD");
    r.git(&["checkout", "-q", "-b", "feature"]);
    // The agent removes bravo; the reviewer questions it on the branch and sends.
    let removed = BASE.replace("fn bravo() -> u32 { 2 }\n", "");
    r.write("lib.rs", &removed);
    let mut app = app_on(&r);
    app.set_scope(Scope::Branch).unwrap();
    open(&mut app, "lib.rs");
    comment_on_row(&mut app, "-fn bravo() -> u32 { 2 }", "who calls bravo?");
    assert_eq!(place(&app), (Side::Old, 2, false));
    assert!(app.export(&common_fake_target()));
    let (baselines, failed) = herdr_reviewr::world::snapshot_all(&app.repo, app.repos());
    app.land_send_baselines(baselines, &failed);
    // The agent's next round touches the file; the reviewer reads just that round, then goes back.
    r.write("lib.rs", &removed.replace("{ 4 }", "{ 40 }"));
    common::land_world(&mut app);
    app.toggle_since_send().unwrap();
    assert_eq!(app.scope, Scope::SinceSend);
    open(&mut app, "lib.rs");
    app.toggle_since_send().unwrap();
    assert_eq!(app.scope, Scope::Branch);
    open(&mut app, "lib.rs");
    assert_eq!(place(&app), (Side::Old, 2, false), "bravo is still the removed line 2");
}

#[test]
fn a_comment_moved_by_an_amend_names_its_line_in_the_new_commit() {
    let r = Repo::init();
    r.write("base.txt", "base\n");
    r.commit_all("base");
    r.write("lib.rs", BASE);
    r.commit_all("Add lib");
    let made = sha(&r, "HEAD");
    let mut app = app_on(&r);
    pick(&mut app, &made);
    open(&mut app, "lib.rs");
    comment_on_row(&mut app, "+fn charlie() -> u32 { 3 }", "why 3?");
    assert_eq!(
        app.store.get(0).unwrap().location(),
        format!("lib.rs:3 (in commit {})", &made[..7])
    );
    // The agent amends: two lines above charlie, which itself reads the same.
    r.write("lib.rs", &BASE.replace("fn charlie", "// a\n// b\nfn charlie"));
    r.git(&["commit", "-q", "-a", "--amend", "--no-edit"]);
    let amended = sha(&r, "HEAD");
    common::land_world(&mut app);
    // What a send names now, before the reviewer opens the amended commit.
    let sent = app.store.get(0).unwrap().location();
    pick(&mut app, &amended);
    open(&mut app, "lib.rs");
    assert_eq!(place(&app), (Side::New, 5, false), "the amended commit shows it on line 5");
    assert_eq!(sent, format!("lib.rs:5 (in commit {})", &amended[..7]), "the send's line");
}

#[test]
fn in_a_folder_an_amend_in_one_clone_moves_only_that_clones_comment() {
    use herdr_reviewr::model::{CommitPick, Picks, Rev};
    let root = tempfile::tempdir().unwrap();
    let api = Repo::init();
    api.write("lib.rs", BASE);
    api.commit_all("base");
    api.write("lib.rs", &BASE.replace("{ 2 }", "{ 22 }"));
    api.commit_all("Tune bravo");
    let made = sha(&api, "HEAD");
    for name in ["one", "two"] {
        let to = root.path().join(name);
        let out = std::process::Command::new("git")
            .args(["clone", "-q", api.path().to_str().unwrap(), to.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(out.status.success());
    }
    let mut app = App::new(root.path().to_path_buf(), Scope::Uncommitted, None);
    app.set_repos(herdr_reviewr::repos::Repos::discover(root.path()));
    app.reload().unwrap();
    app.commit_picks = Picks::from([
        ("one".to_string(), CommitPick::single(&made)),
        ("two".to_string(), CommitPick::single(&made)),
    ]);
    app.set_scope(Scope::Commits).unwrap();
    for name in ["one", "two"] {
        open(&mut app, &format!("{name}/lib.rs"));
        comment_on_row(&mut app, "+fn bravo() -> u32 { 22 }", name);
    }
    // The agent amends in `one` only.
    let one = root.path().join("one");
    std::fs::write(one.join("lib.rs"), BASE.replace("{ 2 }", "{ 23 }")).unwrap();
    let amend = std::process::Command::new("git")
        .args(["-C", one.to_str().unwrap(), "-c", "user.name=T", "-c", "user.email=t@t"])
        .args(["commit", "-q", "-a", "--amend", "--no-edit"])
        .output()
        .unwrap();
    assert!(amend.status.success());
    common::land_world(&mut app);
    let revs: Vec<Rev> = app.store.iter().map(|c| c.rev.clone()).collect();
    assert_ne!(revs[0], Rev::Commit(CommitPick::single(&made)), "one's comment follows the amend");
    assert_eq!(revs[1], Rev::Commit(CommitPick::single(&made)), "two's commit was not rewritten");
}

#[cfg(unix)]
#[test]
fn a_refresh_never_reads_a_commented_link_through_to_its_target() {
    let r = Repo::init();
    r.write("notes.txt", "first note line\nsecond note line\n");
    r.commit_all("base");
    std::os::unix::fs::symlink("notes.txt", r.path().join("link")).unwrap();
    r.write("notes.txt", "first note line\nsecond note line\nthird\n");
    let mut app = app_on(&r);
    open(&mut app, "link");
    comment_on_row(&mut app, "+notes.txt", "why link it?");
    open(&mut app, "notes.txt");
    common::land_world(&mut app);
    let sent = Capture(std::cell::RefCell::new(String::new()));
    assert!(app.export(&sent));
    let text = sent.0.borrow().clone();
    assert!(
        !text.contains("the code changed since"),
        "the link still names notes.txt, as its diff shows:\n{text}"
    );
}

#[test]
fn an_agents_comment_landing_under_the_composer_shows_once_it_closes() {
    let r = Repo::init();
    r.write("lib.rs", BASE);
    r.commit_all("base");
    r.write("lib.rs", &BASE.replace("{ 2 }", "{ 22 }"));
    let mut app = app_on(&r);
    common::enter_tab(&mut app, herdr_reviewr::app::Tab::AllFiles);
    open(&mut app, "lib.rs");
    // The reviewer starts a comment on alpha; the agent comments on bravo meanwhile.
    app.focus = Focus::Diff;
    app.diff_cursor = 0;
    app.start_comment();
    let path = std::fs::canonicalize(r.path().join("lib.rs")).unwrap();
    let number = app.receive_comment(&path, 2, 2, "22 matches the spec".into(), None);
    assert_eq!(number, Ok(1));
    app.cancel_comment();
    app.catch_up_held_view();
    let shown = app.card_rows().iter().any(|&(_, ci)| ci == 0);
    assert!(shown, "the agent's comment on line 2 of the open file never shows");
}

#[test]
fn next_comment_into_a_file_that_shows_none_keeps_the_cursor_on_a_row() {
    let (_r, mut app) = deleted_file_review();
    open(&mut app, "a.rs");
    let (row, ci) = app.card_rows()[0];
    app.focus = Focus::Diff;
    app.diff_cursor = row;
    app.target_comment_card(ci);
    app.jump_comment(1);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 30)).unwrap();
    terminal.draw(|f| herdr_reviewr::ui::render(f, &app)).unwrap();
    assert!(
        app.diff_cursor < app.visible.len(),
        "{:?}: cursor {} on {} rows",
        app.diff_path,
        app.diff_cursor,
        app.visible.len()
    );
}

#[test]
fn a_draft_whose_line_the_agent_reverted_while_it_was_written_goes_out_marked() {
    let (r, mut app, _head, _edited) = two_file_review();
    app.store.take(0);
    open(&mut app, "lib.rs");
    app.focus = Focus::Diff;
    app.diff_cursor = app
        .visible
        .iter()
        .position(|v| v.row().is_some_and(|r| r.text() == "fn inserted_helper() -> u32 { 42 }"))
        .unwrap();
    app.start_comment();
    // While the reviewer types, the agent reverts lib.rs: the commented line is gone.
    r.git(&["checkout", "-q", "--", "lib.rs"]);
    common::land_world(&mut app);
    common::typed(&mut app, "why 42?");
    app.submit_comment();
    app.catch_up_held_view();
    let sent = Capture(std::cell::RefCell::new(String::new()));
    assert!(app.export(&sent));
    let text = sent.0.borrow().clone();
    assert!(text.contains("the code changed since"), "the line it names is gone:\n{text}");
}

#[test]
fn rewrites_that_should_carry_a_comment_do() {
    use herdr_reviewr::git::rewritten;
    use herdr_reviewr::model::CommitPick;
    let one = |s: &str| Some(CommitPick::single(s));
    let r = Repo::init();
    r.write("lib.rs", BASE);
    r.commit_all("base");
    r.set_origin_default("main", "HEAD");
    r.git(&["checkout", "-q", "-b", "feature"]);
    r.write("a.rs", "fn a() {\n    one();\n}\n");
    r.commit_all("wip");
    let wip_a = sha(&r, "HEAD");
    r.write("b.rs", "fn b() {\n    two();\n}\n");
    r.commit_all("wip");
    let wip_b = sha(&r, "HEAD");
    r.write("c.rs", "fn c() {\n    three();\n}\n");
    r.commit_all("Add c");
    let add_c = sha(&r, "HEAD");
    // Upstream moves; the agent rebases onto it, reordering `Add c` first and fixing up the
    // second `wip` with an extra line.
    r.git(&["checkout", "-q", "main"]);
    r.write("up.rs", "fn up() {}\n");
    r.commit_all("Upstream change");
    r.git(&["checkout", "-q", "-B", "feature", "main"]);
    r.git(&["cherry-pick", &add_c]);
    let add_c2 = sha(&r, "HEAD");
    r.git(&["cherry-pick", &wip_a]);
    let wip_a2 = sha(&r, "HEAD");
    r.write("b.rs", "fn b() {\n    two();\n    two_more();\n}\n");
    r.commit_all("wip");
    let fixed = sha(&r, "HEAD");
    assert_eq!(rewritten(r.path(), &CommitPick::single(&add_c)), one(&add_c2), "reordered");
    assert_eq!(rewritten(r.path(), &CommitPick::single(&wip_a)), one(&wip_a2), "shared subject");
    assert_eq!(rewritten(r.path(), &CommitPick::single(&wip_b)), one(&fixed), "fixed up");
}

#[test]
fn a_shared_subject_follows_a_rebase_over_a_busy_main() {
    use herdr_reviewr::git::rewritten;
    use herdr_reviewr::model::CommitPick;
    let r = Repo::init();
    r.write("lib.rs", BASE);
    r.commit_all("base");
    r.git(&["checkout", "-q", "-b", "feature"]);
    r.write("a.rs", "fn a() {\n    one();\n}\n");
    r.commit_all("wip");
    let wip_a = sha(&r, "HEAD");
    r.write("b.rs", "fn b() {\n    two();\n}\n");
    r.commit_all("wip");
    // main gains 65 commits while the agent works; the agent rebases onto it unchanged.
    r.git(&["checkout", "-q", "main"]);
    for n in 0..65 {
        r.write("up.txt", &format!("{n}\n"));
        r.commit_all(&format!("Upstream {n}"));
    }
    r.git(&["checkout", "-q", "feature"]);
    r.git(&["rebase", "-q", "main"]);
    let wip_a2 = sha(&r, "HEAD~1");
    assert_eq!(rewritten(r.path(), &CommitPick::single(&wip_a)), Some(CommitPick::single(&wip_a2)));
}

#[test]
fn an_agents_comment_in_a_fold_landing_under_the_composer_opens_it_once_it_closes() {
    let r = Repo::init();
    let body =
        (1..=40).map(|n| format!("fn line_{n}() -> u32 {{ {n} }}\n")).collect::<Vec<_>>().concat();
    r.write("lib.rs", &body);
    r.commit_all("base");
    r.write("lib.rs", &body.replace("{ 40 }", "{ 400 }"));
    let mut app = app_on(&r);
    open(&mut app, "lib.rs");
    assert!(app.visible.iter().any(|v| v.fold_anchor().is_some()), "line 2 sits in a fold");
    // The reviewer starts a comment on the change; the agent comments on line 2 meanwhile.
    app.focus = Focus::Diff;
    app.diff_cursor =
        app.visible.iter().position(|v| v.row().is_some_and(|r| r.marker() == '+')).unwrap();
    app.start_comment();
    let path = std::fs::canonicalize(r.path().join("lib.rs")).unwrap();
    assert_eq!(app.receive_comment(&path, 2, 2, "line 2 is load-bearing".into(), None), Ok(1));
    app.cancel_comment();
    app.catch_up_held_view();
    let shown = app.card_rows().iter().any(|&(_, ci)| ci == 0);
    // `A` goes to the new comment: it must end up on screen before it counts as read.
    app.jump_answer();
    let after = app.card_rows().iter().any(|&(_, ci)| ci == 0);
    assert!(
        shown && (after || app.store.thread(0).unwrap().unread),
        "hidden in its fold (shown {shown}); after `A` shown {after}, unread {}",
        app.store.thread(0).unwrap().unread
    );
}

/// A send target that always delivers.
fn common_fake_target() -> impl herdr_reviewr::export::ExportTarget {
    struct Delivers;
    impl herdr_reviewr::export::ExportTarget for Delivers {
        fn label(&self) -> &'static str {
            "fake"
        }
        fn reader(&self) -> Option<&str> {
            None
        }
        fn success_message(&self, what: &str) -> String {
            format!("exported {what}")
        }
        fn failure_message(&self, _error: &anyhow::Error, _copy: &str) -> String {
            String::new()
        }
        fn export(&self, _text: &str) -> anyhow::Result<()> {
            Ok(())
        }
    }
    Delivers
}
