//! Threads at their edges: what the dot, `A`, `n`, the list and a send do as the view moves.

mod common;

use std::cell::RefCell;

use anyhow::{Result, bail};
use common::{Repo, UnifiedRow as _, app_on, enter_tab, typed};
use herdr_reviewr::app::{App, Focus, Mode, Tab};
use herdr_reviewr::export::ExportTarget;

/// An export target that records what it was handed and can be made to fail.
struct FakeTarget {
    ok: bool,
    captured: RefCell<Vec<String>>,
}

impl FakeTarget {
    fn ok() -> Self {
        Self { ok: true, captured: RefCell::new(Vec::new()) }
    }
}

impl ExportTarget for FakeTarget {
    fn label(&self) -> &'static str {
        "fake"
    }
    fn success_message(&self, what: &str) -> String {
        format!("exported {what}")
    }
    fn failure_message(&self, _error: &anyhow::Error, _copy: &str) -> String {
        "fake not found".to_string()
    }
    fn export(&self, text: &str) -> Result<()> {
        self.captured.borrow_mut().push(text.to_string());
        if self.ok { Ok(()) } else { bail!("fake export failure") }
    }
}

fn row_with(app: &App, marker: char) -> usize {
    app.visible.iter().position(|r| r.marker() == marker).expect("a row with that marker")
}

fn file_row(app: &App, path: &str) -> usize {
    app.file_rows
        .iter()
        .position(|r| r.file_index().is_some_and(|i| app.entries[i].path == path))
        .expect("a file row for the path")
}

fn comment_on(app: &mut App, marker: char, text: &str) {
    app.focus = Focus::Diff;
    app.diff_cursor = row_with(app, marker);
    app.start_comment();
    typed(app, text);
    app.submit_comment();
}

fn spot(r: &Repo, rel: &str) -> std::path::PathBuf {
    std::fs::canonicalize(r.path().join(rel)).unwrap()
}

/// A deleted file whose thread sits on neither side wears no `●`, and `A` opens the list on it.
#[test]
fn a_thread_on_a_deleted_files_added_line_is_not_promised_by_the_dot_or_a() {
    let r = Repo::init();
    r.write("a.rs", "alpha\nbeta\n");
    r.write("b.rs", "one\n");
    r.commit_all("init");
    r.write("a.rs", "alpha\nbeta\nfn helper_we_do_not_need() {}\n");
    r.write("b.rs", "ONE\n");
    let mut app = app_on(&r);
    app.select_file(file_row(&app, "a.rs")).unwrap();
    comment_on(&mut app, '+', "do we need this file at all?");
    assert!(app.export(&FakeTarget::ok()));

    // The agent agrees: it deletes the file and answers.
    r.remove("a.rs");
    app.select_file(file_row(&app, "b.rs")).unwrap();
    common::land_world(&mut app);
    app.receive_reply(1, "No: deleted it.".into(), None);

    // Opening a.rs shows no thread: nothing promises one there.
    let promised = app.has_new_answer("a.rs");
    app.jump_answer();
    let landed = app.diff_path.clone();
    let cards = app.card_rows();
    assert!(
        !promised && app.mode == Mode::List,
        "dot on a.rs: {promised}; A opened {landed:?} with cards {cards:?}, mode {:?}, \
         status {:?}, unread now {}",
        app.mode,
        app.status,
        app.store.unread()
    );
}

/// So does a file emptied under its thread: no line holds it, so no mark promises it.
#[test]
fn a_thread_on_an_emptied_file_is_not_promised_by_the_dot_or_a() {
    let r = Repo::init();
    r.write("a.rs", "alpha\nbeta\n");
    r.write("b.rs", "one\n");
    r.commit_all("init");
    r.write("a.rs", "alpha\nbeta\nfn helper_we_do_not_need() {}\n");
    r.write("b.rs", "ONE\n");
    let mut app = app_on(&r);
    app.select_file(file_row(&app, "a.rs")).unwrap();
    comment_on(&mut app, '+', "do we need any of this?");
    assert!(app.export(&FakeTarget::ok()));
    r.write("a.rs", "");
    app.select_file(file_row(&app, "b.rs")).unwrap();
    common::land_world(&mut app);
    app.receive_reply(1, "No: emptied it.".into(), None);
    let promised = app.has_new_answer("a.rs");
    app.jump_answer();
    let (landed, cards) = (app.diff_path.clone(), app.card_rows());
    assert!(
        !promised && app.mode == Mode::List,
        "dot on a.rs: {promised}; A opened {landed:?} with cards {cards:?}, status {:?}",
        app.status
    );
}

/// An agent's comment that lands while the list freezes the view shows once the list closes.
#[test]
fn an_agents_comment_landing_under_the_list_shows_once_the_list_closes() {
    let r = Repo::init();
    r.write("a.rs", "alpha\nbeta\ngamma\n");
    r.commit_all("init");
    r.write("a.rs", "alpha\nBETA_CONSTANT_NAME\ngamma\n");
    let mut app = app_on(&r);
    enter_tab(&mut app, Tab::AllFiles);
    app.select_file(file_row(&app, "a.rs")).unwrap();
    // A comment of the reviewer's own, so the list opens.
    app.focus = Focus::Diff;
    app.diff_cursor = 0;
    app.start_comment();
    typed(&mut app, "first line note");
    app.submit_comment();

    // Control: landing outside a modal, the agent's comment shows at once in the open file.
    let mut control = app_on(&r);
    enter_tab(&mut control, Tab::AllFiles);
    control.select_file(file_row(&control, "a.rs")).unwrap();
    let number = control.receive_comment(&spot(&r, "a.rs"), 2, 2, "renamed".into(), None).unwrap();
    let shown = control.card_rows().iter().any(|&(_, ci)| control.store.number(ci) == Some(number));
    assert!(shown, "control: the open file shows the agent's card");

    // Under the list, the same comment lands frozen; the list then closes.
    app.open_list();
    let number = app.receive_comment(&spot(&r, "a.rs"), 2, 2, "renamed".into(), None).unwrap();
    app.close_list();
    app.catch_up_held_view();
    let shown = app.card_rows().iter().any(|&(_, ci)| app.store.number(ci) == Some(number));
    let dot = app.has_new_answer("a.rs");
    app.jump_answer();
    assert!(
        shown && dot && app.mode == Mode::Normal,
        "card shown: {shown}, dot: {dot}, A left mode {:?} with status {:?}",
        app.mode,
        app.status
    );
}

fn render(app: &App, width: u16, height: u16) -> String {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| herdr_reviewr::ui::render(f, app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    let mut out = String::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            out.push_str(buffer.cell((x, y)).map_or(" ", |c| c.symbol()));
        }
        out.push('\n');
    }
    out
}

/// A reply to a thread the open view doesn't show opens its box over the body, under the thread.
#[test]
fn a_reply_started_from_the_list_always_shows_its_box() {
    let r = Repo::init();
    r.write("a.rs", "alpha\nbeta\n");
    r.commit_all("init");
    r.write("a.rs", "alpha\nBETA\n");
    let mut app = app_on(&r);
    comment_on(&mut app, '+', "why upper case?");
    assert!(app.export(&FakeTarget::ok()));
    r.commit_all("agent: commit the work");
    common::land_world(&mut app);
    assert!(app.visible.is_empty(), "uncommitted is empty now");
    app.receive_reply(1, "It names a constant.".into(), None);
    app.jump_answer();
    assert_eq!(app.mode, Mode::List, "{}", app.status);
    app.start_reply();
    assert!(app.composing() && app.replying_to() == Some(1));
    typed(&mut app, "then name it so");
    let out = render(&app, 120, 30);
    assert!(
        out.contains("reply #1") && out.contains("then name it so"),
        "the reply box and its text are on screen:\n{out}"
    );
}

/// Cards, the list and a reply box paint at every size down to 1×1.
#[test]
fn threads_paint_at_every_small_size() {
    let r = Repo::init();
    r.write("a.rs", "alpha\nbeta\n");
    r.commit_all("init");
    r.write("a.rs", "alpha\nBETA\n");
    let mut app = app_on(&r);
    comment_on(&mut app, '+', "why upper case? 日本語の説明 with a long tail of words");
    assert!(app.export(&FakeTarget::ok()));
    let answer = (1..=12)
        .map(|n| format!("step {n}: 長い答え {}", "x".repeat(n * 7)))
        .collect::<Vec<_>>()
        .concat();
    app.receive_reply(1, answer, Some("a-very-long-agent-name-that-runs-on".into()));
    for _ in 0..3 {
        comment_on(&mut app, '-', "and this?");
    }
    let mut shapes: Vec<(&str, App)> = Vec::new();
    let mut list = app_on(&r);
    std::mem::swap(&mut list, &mut app);
    list.open_list();
    list.list_move(10);
    shapes.push(("list", list));
    // The same thread in the diff, and a reply being written under it.
    let build = || {
        let mut app = app_on(&r);
        comment_on(&mut app, '+', "why upper case? 日本語の説明 with a long tail of words");
        assert!(app.export(&FakeTarget::ok()));
        let answer = (1..=12)
            .map(|n| format!("step {n}: 長い答え {}\n", "x".repeat(n * 7)))
            .collect::<Vec<_>>()
            .concat();
        app.receive_reply(1, answer, Some("a-very-long-agent-name-that-runs-on".into()));
        app
    };
    shapes.push(("diff", build()));
    let mut replying = build();
    replying.focus = Focus::Diff;
    replying.diff_cursor = row_with(&replying, '+');
    replying.start_reply();
    typed(&mut replying, "a reply that wraps 日本語 ".repeat(6).as_str());
    shapes.push(("reply", replying));
    for (name, app) in &shapes {
        for w in 1..=60u16 {
            for h in 1..=24u16 {
                let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    render(app, w, h);
                }));
                assert!(caught.is_ok(), "{name} panicked at {w}x{h}");
            }
        }
    }
}

/// A removed line counts in its own scope's base: a look at another scope never moves it.
#[test]
fn a_removed_side_thread_keeps_its_line_across_a_look_at_another_scope() {
    let r = Repo::init();
    r.write("a.rs", "fn alpha() {}\nfn gamma() {}\n");
    r.commit_all("base");
    r.git(&["checkout", "-q", "-b", "feature"]);
    r.write("a.rs", "fn alpha() {}\nfn beta_was_added_on_the_branch() {}\nfn gamma() {}\n");
    r.commit_all("add beta");
    r.write("a.rs", "fn alpha() {}\nfn gamma() {}\nfn delta() {}\n");
    let mut app = herdr_reviewr::app::App::new(
        r.path_buf(),
        herdr_reviewr::model::Scope::Uncommitted,
        Some("main".into()),
    );
    app.reload().unwrap();
    comment_on(&mut app, '-', "why drop beta?");
    assert!(app.export(&FakeTarget::ok()));
    let before = app.store.get(0).unwrap().location();
    assert_eq!(before, "a.rs:2 (removed)");
    assert!(!app.store.changed(0));

    app.set_scope(herdr_reviewr::model::Scope::Branch).unwrap();
    app.set_scope(herdr_reviewr::model::Scope::Uncommitted).unwrap();
    let after = app.store.get(0).unwrap().location();
    let place = app.card_place(0);
    assert!(
        after == before && !app.store.changed(0),
        "after a look at branch: {after}, code changed: {}, card {place:?}",
        app.store.changed(0)
    );
}

/// Nor does a round through `since send` once the agent edited the file.
#[test]
fn a_removed_side_thread_keeps_its_line_across_since_send_and_back() {
    let r = Repo::init();
    r.write("a.rs", "fn alpha() {}\nfn beta_line_here() {}\nfn gamma() {}\n");
    r.commit_all("base");
    r.write("a.rs", "fn alpha() {}\nfn gamma() {}\n");
    let mut app = app_on(&r);
    comment_on(&mut app, '-', "why drop beta?");
    assert!(app.export(&FakeTarget::ok()));
    let (baselines, failed) = herdr_reviewr::world::snapshot_all(&app.repo, app.repos());
    app.land_send_baselines(baselines, &failed);
    let before = app.store.get(0).unwrap().location();
    assert_eq!(before, "a.rs:2 (removed)");

    // The agent's follow-up touches another line of the same file.
    r.write("a.rs", "fn alpha() {}\nfn gamma() {}\nfn delta() {}\n");
    common::land_world(&mut app);
    assert_eq!(app.store.get(0).unwrap().location(), before, "uncommitted still reads it");
    app.toggle_since_send().unwrap();
    app.toggle_since_send().unwrap();
    assert_eq!(app.scope, herdr_reviewr::model::Scope::Uncommitted);
    let after = app.store.get(0).unwrap().location();
    assert!(
        after == before && !app.store.changed(0),
        "after i and back: {after}, code changed: {}",
        app.store.changed(0)
    );
}

/// A fence closes only on a bare run of its marker at least as long, so a nested one keeps its blanks.
#[test]
fn a_send_keeps_the_blank_lines_of_a_fence_that_holds_another() {
    use herdr_reviewr::export::{Outgoing, format_send};
    use herdr_reviewr::model::{Comment, Rev, Side};
    let c = Comment {
        file: "README.md".into(),
        side: Side::New,
        start: 1,
        end: 1,
        lines: "+x".into(),
        text: String::new(),
        diff_anchored: true,
        rev: Rev::Worktree,
    };
    let text = "Write it so:\n````md\n```rust\nfn a() {}\n\nfn b() {}\n```\n````";
    let sent = format_send(None, &[Outgoing::Reply(1, &c, text)], None);
    assert!(sent.contains("fn a() {}\n\nfn b() {}"), "the code's own blank line stays:\n{sent}");
}

/// Back on a tab before its refresh lands, a comment holds its own file's lines.
#[test]
fn a_comment_made_right_after_a_tab_switch_holds_its_own_files_lines() {
    let r = Repo::init();
    r.write("a.rs", "fn a_one() {}\nfn a_two() {}\n");
    r.write("b.rs", "fn b_one() {}\nfn b_two() {}\n");
    r.commit_all("init");
    r.write("a.rs", "fn a_one() {}\nfn a_two_renamed() {}\n");
    r.write("b.rs", "fn b_one() {}\nfn b_two_renamed() {}\n");
    let mut app = app_on(&r);
    app.select_file(file_row(&app, "a.rs")).unwrap();
    enter_tab(&mut app, Tab::AllFiles);
    app.select_file(file_row(&app, "b.rs")).unwrap();
    // Back to Changes: the stash brings a.rs back; its refresh has not landed yet.
    app.set_tab(Tab::Changes).unwrap();
    assert_eq!(app.diff_path.as_deref(), Some("a.rs"));
    comment_on(&mut app, '+', "why rename?");
    let held = app.store.held(0).to_vec();
    common::land_world(&mut app);
    assert!(
        held == ["fn a_two_renamed() {}"] && !app.store.changed(0),
        "held {held:?}, code changed after the refresh: {}",
        app.store.changed(0)
    );
}

/// `n` never crosses into a file whose opening shows no card.
#[test]
fn next_comment_never_strands_the_reviewer_in_a_file_that_shows_none() {
    let r = Repo::init();
    r.write("a.rs", "alpha\nbeta\n");
    r.write("b.rs", "one\ntwo\nthree\nfour\n");
    r.commit_all("init");
    r.write("a.rs", "alpha\nbeta\nfn helper_we_do_not_need() {}\n");
    r.write("b.rs", "one\ntwo\nthree\nFOUR\n");
    let mut app = app_on(&r);
    app.select_file(file_row(&app, "a.rs")).unwrap();
    comment_on(&mut app, '+', "do we need this file at all?");
    app.select_file(file_row(&app, "b.rs")).unwrap();
    comment_on(&mut app, '+', "why upper case?");
    assert!(app.export(&FakeTarget::ok()));
    r.remove("a.rs");
    common::land_world(&mut app);
    // The reviewer sits on b.rs's card (rows: one two three -four +FOUR; its card under row 4).
    app.select_file(file_row(&app, "b.rs")).unwrap();
    app.focus = Focus::Diff;
    app.diff_cursor = 4;
    app.target_comment_card(1);
    assert_eq!(app.card_rows(), [(4, 1)], "on b's card");

    app.jump_comment(1);
    let (path, cursor, rows) = (app.diff_path.clone(), app.diff_cursor, app.visible.len());
    let cards = app.card_rows();
    assert!(
        path.as_deref() == Some("b.rs") || cards.iter().any(|&(row, _)| row == cursor),
        "n left the reviewer in {path:?} at row {cursor} of {rows}, cards there: {cards:?}"
    );
}

/// An agent's comment landing while the reviewer types shows in a commit's file once the box closes.
#[test]
fn an_agents_comment_on_a_commits_file_shows_once_the_composer_closes() {
    let r = Repo::init();
    r.write("a.rs", "fn one() {}\nfn three() {}\n");
    r.commit_all("base");
    r.write("a.rs", "fn one() {}\nfn two_is_new_here() {}\nfn three() {}\n");
    r.commit_all("add two");
    let mut app = app_on(&r);
    app.open_commit_picker();
    app.commit_picker_pick().unwrap();
    assert_eq!(app.scope, herdr_reviewr::model::Scope::Commits);
    app.select_file(file_row(&app, "a.rs")).unwrap();
    // Control: outside a modal, the open commit shows the agent's comment on its line.
    let mut control = app_on(&r);
    control.open_commit_picker();
    control.commit_picker_pick().unwrap();
    control.select_file(file_row(&control, "a.rs")).unwrap();
    control.receive_comment(&spot(&r, "a.rs"), 2, 2, "new on purpose".into(), None).unwrap();
    assert_eq!(control.card_rows().len(), 1, "control: the card shows");

    app.focus = Focus::Diff;
    app.diff_cursor = row_with(&app, '+');
    app.start_comment();
    app.receive_comment(&spot(&r, "a.rs"), 2, 2, "new on purpose".into(), None).unwrap();
    typed(&mut app, "is two tested?");
    app.submit_comment();
    app.catch_up_held_view();
    let agents = app.card_rows().iter().filter(|&&(_, ci)| ci == 0).count();
    assert_eq!(agents, 1, "the agent's card shows beside the reviewer's: {:?}", app.card_rows());
}

/// One on a folded line opens its fold once the box closes.
#[test]
fn an_agents_comment_on_a_folded_line_shows_once_the_composer_closes() {
    let r = Repo::init();
    let base = (1..=20)
        .map(|n| format!("let value_{n:02} = compute({n});\n"))
        .collect::<Vec<_>>()
        .concat();
    r.write("a.rs", &base);
    r.commit_all("init");
    r.write("a.rs", &base.replacen("compute(1)", "compute_fast(1)", 1));
    let mut app = app_on(&r);
    // Control: outside a modal, the fold holding the line opens and the card shows.
    let mut control = app_on(&r);
    control.receive_comment(&spot(&r, "a.rs"), 12, 12, "unused now".into(), None).unwrap();
    assert_eq!(control.card_rows().len(), 1, "control: the card shows");

    app.focus = Focus::Diff;
    app.diff_cursor = row_with(&app, '+');
    app.start_comment();
    typed(&mut app, "half a tho");
    app.receive_comment(&spot(&r, "a.rs"), 12, 12, "unused now".into(), None).unwrap();
    app.cancel_comment();
    app.catch_up_held_view();
    let cards = app.card_rows();
    app.jump_answer();
    let cursor_on_card = app.card_rows().iter().any(|&(row, _)| row == app.diff_cursor);
    assert!(
        !cards.is_empty() && cursor_on_card,
        "cards after the composer closed: {cards:?}; after A: cursor {} on a card: \
         {cursor_on_card}, unread {}",
        app.diff_cursor,
        app.store.unread()
    );
}

/// A thread reopened from the list brings its card back out of the fold it closed into.
#[test]
fn reopening_a_thread_from_the_list_brings_its_card_back_out_of_a_fold() {
    let r = Repo::init();
    let base = (1..=20)
        .map(|n| format!("let value_{n:02} = compute({n});\n"))
        .collect::<Vec<_>>()
        .concat();
    r.write("a.rs", &base);
    r.write("b.rs", "one\n");
    r.commit_all("init");
    r.write("a.rs", &base.replacen("compute(1)", "compute_fast(1)", 1));
    r.write("b.rs", "ONE\n");
    let mut app = app_on(&r);
    app.select_file(file_row(&app, "a.rs")).unwrap();
    app.receive_comment(&spot(&r, "a.rs"), 12, 12, "unused now".into(), None).unwrap();
    let (row, _) = app.card_rows()[0];
    app.focus = Focus::Diff;
    app.diff_cursor = row;
    app.toggle_resolved();
    assert!(app.card_rows().is_empty(), "resolved: out of the diff");
    // Away and back: the fold that held it closes.
    app.select_file(file_row(&app, "b.rs")).unwrap();
    app.select_file(file_row(&app, "a.rs")).unwrap();
    app.open_list();
    app.toggle_resolved();
    assert_eq!(app.status, "#1 reopened");
    app.close_list();
    app.catch_up_held_view();
    assert_eq!(app.card_rows().len(), 1, "reopened, its card is back in the diff");
}

/// So does a reply, which reopens a resolved thread.
#[test]
fn replying_on_a_resolved_thread_brings_its_card_back_out_of_a_fold() {
    let r = Repo::init();
    let base = (1..=20)
        .map(|n| format!("let value_{n:02} = compute({n});\n"))
        .collect::<Vec<_>>()
        .concat();
    r.write("a.rs", &base);
    r.write("b.rs", "one\n");
    r.commit_all("init");
    r.write("a.rs", &base.replacen("compute(1)", "compute_fast(1)", 1));
    r.write("b.rs", "ONE\n");
    let mut app = app_on(&r);
    app.select_file(file_row(&app, "a.rs")).unwrap();
    app.receive_comment(&spot(&r, "a.rs"), 12, 12, "unused now".into(), None).unwrap();
    let (row, _) = app.card_rows()[0];
    app.focus = Focus::Diff;
    app.diff_cursor = row;
    app.toggle_resolved();
    app.select_file(file_row(&app, "b.rs")).unwrap();
    app.select_file(file_row(&app, "a.rs")).unwrap();
    app.open_list();
    app.start_reply();
    typed(&mut app, "actually, is it?");
    app.submit_comment();
    assert!(!app.store.thread(0).unwrap().resolved, "the reply reopened it");
    app.close_list();
    app.catch_up_held_view();
    assert_eq!(app.card_rows().len(), 1, "reopened by the reply, its card is back in the diff");
}
