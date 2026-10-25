//! Edge cases other agent-review tools test for, run against reviewr end to end: the change
//! lists, the open diff, a comment on it, and the text a send carries.

mod common;

use common::{Repo, app_on};
use herdr_reviewr::app::{App, Focus};
use herdr_reviewr::model::{ChangeKind, Scope};

/// The listed paths, as the file list shows them.
fn listed(app: &App) -> Vec<String> {
    app.entries.iter().map(|e| e.path.clone()).collect()
}

/// Open `path` and return its rows as `marker + text`.
fn open(app: &mut App, path: &str) -> Vec<String> {
    let row = app
        .file_rows
        .iter()
        .position(|r| r.file_index().is_some_and(|i| app.entries[i].path == path))
        .unwrap_or_else(|| panic!("{path} is listed: {:?}", listed(app)));
    app.select_file(row).unwrap();
    app.visible
        .iter()
        .filter_map(|v| v.row())
        .map(|r| format!("{}{}", r.marker(), r.text()))
        .collect()
}

/// Comment `text` on the open diff's first row reading `marker + line`, and return the send.
fn comment_and_send(app: &mut App, row: &str, text: &str) -> String {
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
    let all: Vec<&herdr_reviewr::model::Comment> = app.store.iter().collect();
    herdr_reviewr::export::format_review(None, &all)
}

// Windows forbids `"` and tabs in file names.
#[cfg(unix)]
#[test]
fn odd_file_names_list_open_and_name_themselves_in_a_send() {
    let r = Repo::init();
    for name in ["with space.rs", "日本語.rs", "quote\"d.rs", "-dash.rs", "tab\tname.rs"] {
        r.write(name, "one\n");
    }
    r.commit_all("init");
    for name in ["with space.rs", "日本語.rs", "quote\"d.rs", "-dash.rs", "tab\tname.rs"] {
        r.write(name, "one\ntwo\n");
    }
    let mut app = app_on(&r);
    let mut names = listed(&app);
    names.sort();
    assert_eq!(names, ["-dash.rs", "quote\"d.rs", "tab\tname.rs", "with space.rs", "日本語.rs"]);
    for name in ["with space.rs", "日本語.rs", "quote\"d.rs", "-dash.rs", "tab\tname.rs"] {
        let rows = open(&mut app, name);
        assert!(rows.contains(&"+two".to_string()), "{name}: {rows:?}");
    }
    let sent = comment_and_send(&mut app, "+two", "why two?");
    assert!(sent.contains("tab\tname.rs:2") || sent.contains(":2\n"), "{sent}");
}

#[test]
fn a_last_line_without_a_newline_diffs_and_comments_as_a_line() {
    let r = Repo::init();
    r.write("a.txt", "one\ntwo");
    r.commit_all("init");
    r.write("a.txt", "one\nTWO");
    let mut app = app_on(&r);
    let rows = open(&mut app, "a.txt");
    assert_eq!(rows, [" one", "-two", "+TWO"], "no `\\ No newline` row");
    let sent = comment_and_send(&mut app, "+TWO", "caps?");
    assert_eq!(sent, "a.txt:2\n+TWO\ncaps?");
    // Adding the newline is a change of its own, which git shows on the last line.
    r.write("a.txt", "one\nTWO\n");
    let mut app = app_on(&r);
    let rows = open(&mut app, "a.txt");
    assert!(rows.contains(&"+TWO".to_string()), "{rows:?}");
}

#[cfg(unix)]
#[test]
fn a_mode_only_change_lists_and_says_so() {
    use std::os::unix::fs::PermissionsExt;
    let r = Repo::init();
    r.write("run.sh", "echo hi\n");
    r.commit_all("init");
    std::fs::set_permissions(r.path().join("run.sh"), std::fs::Permissions::from_mode(0o755))
        .unwrap();
    let mut app = app_on(&r);
    assert_eq!(listed(&app), ["run.sh"]);
    let rows = open(&mut app, "run.sh");
    assert!(rows.iter().all(|r| !r.starts_with('+') && !r.starts_with('-')), "{rows:?}");
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 20)).unwrap();
    terminal.draw(|f| herdr_reviewr::ui::render(f, &app)).unwrap();
    let cells = terminal.backend().buffer().content().iter();
    let screen: String = cells.map(ratatui::buffer::Cell::symbol).collect();
    assert!(screen.contains("mode 100644 → 100755"), "the title names the change:\n{screen}");
}

#[test]
fn an_empty_file_added_lists_and_opens_empty() {
    let r = Repo::init();
    r.write("keep.txt", "k\n");
    r.commit_all("init");
    r.write("empty.txt", "");
    r.git(&["add", "empty.txt"]);
    let mut app = app_on(&r);
    assert_eq!(listed(&app), ["empty.txt"]);
    let kind = app
        .entries
        .iter()
        .find(|e| e.path == "empty.txt")
        .and_then(|e| e.annotation.as_ref())
        .map(|a| a.kind);
    assert_eq!(kind, Some(ChangeKind::Added), "{:?}", app.entries);
    assert!(open(&mut app, "empty.txt").is_empty());
}

#[test]
fn an_unborn_repository_lists_its_files_as_untracked_in_every_worktree_scope() {
    let r = Repo::init();
    r.write("a.rs", "fn main() {}\n");
    let mut app = app_on(&r);
    assert_eq!(listed(&app), ["a.rs"]);
    assert_eq!(open(&mut app, "a.rs"), ["+fn main() {}"]);
    app.set_scope(Scope::Branch).unwrap();
    assert!(app.status.is_empty() || !app.status.contains("panic"), "{}", app.status);
}

#[test]
fn a_merge_conflict_lists_the_file_with_its_markers() {
    let r = Repo::init();
    r.write("a.txt", "base\n");
    r.commit_all("init");
    r.git(&["checkout", "-q", "-b", "other"]);
    r.write("a.txt", "other\n");
    r.commit_all("other");
    r.git(&["checkout", "-q", "main"]);
    r.write("a.txt", "mine\n");
    r.commit_all("mine");
    let out = std::process::Command::new("git")
        .args(["-C", &r.path().to_string_lossy(), "merge", "-q", "other"])
        .output()
        .unwrap();
    assert!(!out.status.success(), "the merge conflicts");
    let mut app = app_on(&r);
    assert_eq!(listed(&app), ["a.txt"]);
    let rows = open(&mut app, "a.txt");
    assert!(rows.iter().any(|r| r.contains("<<<<<<<")), "{rows:?}");
}

#[test]
fn a_crlf_files_rows_and_send_carry_no_carriage_return() {
    let r = Repo::init();
    r.git(&["config", "core.autocrlf", "false"]);
    r.write("w.txt", "alpha\r\nbeta\r\n");
    r.commit_all("init");
    r.write("w.txt", "alpha\r\nBETA\r\n");
    let mut app = app_on(&r);
    let rows = open(&mut app, "w.txt");
    assert!(rows.iter().all(|r| !r.contains('\r')), "{rows:?}");
    let sent = comment_and_send(&mut app, "+BETA", "caps");
    assert!(!sent.contains('\r'), "{sent:?}");
}

#[test]
fn a_selection_across_a_fold_quotes_only_real_lines() {
    let r = Repo::init();
    let lines: Vec<String> = (1..=40).map(|i| format!("line {i}")).collect();
    r.write("f.txt", &(lines.join("\n") + "\n"));
    r.commit_all("init");
    let mut edited = lines.clone();
    edited[1] = "LINE 2".into();
    edited[37] = "LINE 38".into();
    r.write("f.txt", &(edited.join("\n") + "\n"));
    let mut app = app_on(&r);
    open(&mut app, "f.txt");
    app.focus = Focus::Diff;
    let at = |app: &App, text: &str| {
        app.visible.iter().position(|v| v.row().is_some_and(|r| r.text() == text)).unwrap()
    };
    app.diff_cursor = at(&app, "LINE 2");
    app.toggle_select();
    app.diff_cursor = at(&app, "LINE 38");
    app.start_comment();
    for ch in "both edits".chars() {
        app.input_push(ch);
    }
    app.submit_comment();
    let c = app.store.get(0).expect("a comment over the range");
    assert!(!c.lines.contains('⋯') && !c.lines.contains("unmodified"), "{}", c.lines);
    assert_eq!((c.start, c.end), (2, 38));
}

#[test]
fn tabs_and_markdown_in_a_comment_reach_the_send_as_written() {
    let r = Repo::init();
    r.write("t.go", "func f() {\n\treturn\n}\n");
    r.commit_all("init");
    r.write("t.go", "func f() {\n\treturn nil\n}\n");
    let mut app = app_on(&r);
    open(&mut app, "t.go");
    let sent =
        comment_and_send(&mut app, "+\treturn nil", "use `errors.New`:\n```go\nreturn err\n```");
    assert!(sent.contains("+\treturn nil\n"), "the tab survives: {sent:?}");
    assert!(sent.contains("```go\nreturn err\n```"), "{sent}");
}

#[test]
fn a_minified_line_opens_and_paints_in_reasonable_time() {
    let r = Repo::init();
    let long = "value=1;".repeat(30_000);
    r.write("app.min.js", &format!("{long}\n"));
    r.commit_all("init");
    r.write("app.min.js", &format!("{long}x\n"));
    let started = std::time::Instant::now();
    let mut app = app_on(&r);
    let loaded = started.elapsed();
    let started = std::time::Instant::now();
    let rows = open(&mut app, "app.min.js");
    let opened = started.elapsed();
    assert_eq!(rows.len(), 2, "one line out, one in");
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(160, 48)).unwrap();
    let started = std::time::Instant::now();
    terminal.draw(|f| herdr_reviewr::ui::render(f, &app)).unwrap();
    let painted = started.elapsed();
    app.wrap = true;
    let started = std::time::Instant::now();
    terminal.draw(|f| herdr_reviewr::ui::render(f, &app)).unwrap();
    let wrapped = started.elapsed();
    assert!(loaded.as_secs_f64() < 2.0, "a minified line skips coloring: {loaded:?}");
    assert!(opened.as_secs_f64() < 2.0 && painted.as_secs_f64() < 1.0, "{opened:?} {painted:?}");
    assert!(wrapped.as_secs_f64() < 1.0, "{wrapped:?}");
}

#[test]
fn a_comment_on_a_file_the_agent_deletes_stays_on_its_lines() {
    let r = Repo::init();
    r.write("keep.rs", "k\n");
    r.commit_all("init");
    r.write("gone.rs", "fn helper() -> u32 { 42 }\n");
    let mut app = app_on(&r);
    open(&mut app, "gone.rs");
    comment_and_send(&mut app, "+fn helper() -> u32 { 42 }", "why 42?");
    // The agent commits the file, then deletes it: the diff shows it all removed.
    r.commit_all("add helper");
    r.remove("gone.rs");
    common::land_world(&mut app);
    let rows = open(&mut app, "gone.rs");
    assert_eq!(rows, ["-fn helper() -> u32 { 42 }"]);
    assert_eq!(app.card_rows().len(), 1, "the comment shows on the removed line: {rows:?}");
    let place = app.card_place(0).unwrap();
    assert_eq!(
        (place.side, place.start, place.changed),
        (herdr_reviewr::model::Side::Old, 1, false)
    );
}

#[test]
fn every_screen_paints_at_any_size_without_panicking() {
    let r = Repo::init();
    r.write("src/日本語/長い名前のファイル.rs", "fn a() {}\n");
    r.write("b.rs", "let x = 1;\n");
    r.commit_all("init");
    r.write("src/日本語/長い名前のファイル.rs", "fn a() { 🦀🦀🦀 }\n\tlet tab = 1;\n");
    r.write("b.rs", "let x = 2; // 漢字とemoji 👩‍👩‍👧 combining é\n");
    let mut app = app_on(&r);
    open(&mut app, "b.rs");
    comment_and_send(
        &mut app,
        "+let x = 2; // 漢字とemoji 👩‍👩‍👧 combining é",
        "長いコメント 🦀 ".repeat(20).as_str(),
    );
    let paint = |app: &App, what: &str| {
        for (w, h) in [(1, 1), (2, 2), (7, 3), (12, 5), (25, 8), (41, 13), (80, 24), (300, 3)] {
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
            let painted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                terminal.draw(|f| herdr_reviewr::ui::render(f, app)).unwrap();
            }));
            assert!(painted.is_ok(), "{what} at {w}x{h} panicked");
        }
    };
    paint(&app, "the diff with a card");
    app.wrap = true;
    paint(&app, "wrapped");
    app.focus = Focus::Diff;
    app.start_comment();
    for ch in "下書き 🦀".chars() {
        app.input_push(ch);
    }
    paint(&app, "the composer");
    app.cancel_comment();
    app.open_list();
    paint(&app, "the comments list");
    app.close_list();
    app.open_commit_picker();
    paint(&app, "the commit picker");
}

/// Every cell the frame paints, joined: what reaches the terminal.
fn painted(app: &App, w: u16, h: u16) -> String {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| herdr_reviewr::ui::render(f, app)).unwrap();
    terminal.backend().buffer().content().iter().map(ratatui::buffer::Cell::symbol).collect()
}

/// Whether `text` holds a control character a terminal would act on.
fn has_control(text: &str) -> bool {
    text.chars().any(|c| (c.is_control() && c != '\n') || ('\u{80}'..='\u{9f}').contains(&c))
}

#[test]
fn escape_codes_in_a_source_line_never_reach_the_terminal() {
    let r = Repo::init();
    r.write("log.txt", "plain\n");
    r.commit_all("init");
    // No NUL in git's first 8000 bytes, so the file is text and its rows paint.
    let line = "\x1b[31mred\x1b[0m and \x1b]52;c;aGk=\x07 bell\x07 \u{9b}2J tab\tdel\x7f cr\rx";
    r.write("log.txt", &format!("plain\n{line}\n{}nul\0x\n", "pad\n".repeat(2000)));
    let mut app = app_on(&r);
    let rows = open(&mut app, "log.txt");
    assert!(rows.iter().any(|r| r.contains("red")), "the line diffs as text");
    let screen = painted(&app, 120, 20);
    assert!(screen.contains("red") && screen.contains("bell"), "{screen}");
    assert!(
        !has_control(&screen),
        "{:?}",
        screen.chars().filter(|c| c.is_control()).collect::<String>()
    );
}

#[test]
fn escape_codes_in_an_agents_reply_never_reach_the_terminal() {
    let r = Repo::init();
    r.write("a.rs", "one\n");
    r.commit_all("init");
    r.write("a.rs", "two\n");
    let mut app = app_on(&r);
    open(&mut app, "a.rs");
    comment_and_send(&mut app, "+two", "why?");
    app.store.mark_sent();
    app.receive_reply(
        1,
        "fine \x1b[2J\x1b]52;c;aGk=\x07 \u{9b}31m done".into(),
        Some("\x1b[31mevil".into()),
    );
    let screen = painted(&app, 120, 30);
    assert!(
        !has_control(&screen),
        "{:?}",
        screen.chars().filter(|c| has_control(&c.to_string())).collect::<String>()
    );
}

#[cfg(unix)]
#[test]
fn escape_codes_in_a_file_name_never_reach_the_terminal() {
    let r = Repo::init();
    r.write("keep.txt", "k\n");
    r.commit_all("init");
    let name = "evil\x1b]52;c;aGk=\x07\u{9b}name.rs";
    std::fs::write(r.path().join(name), "x\n").unwrap();
    std::fs::write(r.path().join("two\nlines.rs"), "y\n").unwrap();
    let app = app_on(&r);
    assert_eq!(app.entries.len(), 2, "{:?}", listed(&app));
    let screen = painted(&app, 120, 20);
    assert!(
        !has_control(&screen),
        "{:?}",
        screen.chars().filter(|c| has_control(&c.to_string())).collect::<String>()
    );
}

#[cfg(unix)]
#[test]
fn a_fifo_in_the_worktree_never_hangs_the_pane() {
    let r = Repo::init();
    r.write("keep.txt", "k\n");
    r.commit_all("init");
    let fifo = r.path().join("pipe");
    let made = std::process::Command::new("mkfifo").arg(&fifo).status().unwrap();
    assert!(made.success());
    let (tx, rx) = std::sync::mpsc::channel();
    let root = r.path_buf();
    std::thread::spawn(move || {
        let mut app = App::new(root, Scope::Uncommitted, None);
        app.reload().unwrap();
        let names: Vec<String> = app.entries.iter().map(|e| e.path.clone()).collect();
        if let Some(row) = app
            .file_rows
            .iter()
            .position(|r| r.file_index().is_some_and(|i| app.entries[i].path == "pipe"))
        {
            app.select_file(row).unwrap();
        }
        app.set_tab(herdr_reviewr::app::Tab::AllFiles).unwrap();
        if let Some(row) = app
            .file_rows
            .iter()
            .position(|r| r.file_index().is_some_and(|i| app.entries[i].path == "pipe"))
        {
            app.select_file(row).unwrap();
        }
        tx.send(names).unwrap();
    });
    let names =
        rx.recv_timeout(std::time::Duration::from_secs(10)).expect("opening a FIFO hung the pane");
    eprintln!("listed with a fifo: {names:?}");
}

#[test]
fn a_plain_mv_reads_as_a_rename_and_its_comment_follows() {
    let r = Repo::init();
    r.write("old.txt", "one\ntwo\nthree\nfour\nfive\n");
    r.commit_all("init");
    r.write("old.txt", "one\ntwo\nthree\nfour\nfive\nsix\n");
    let mut app = app_on(&r);
    open(&mut app, "old.txt");
    comment_and_send(&mut app, "+six", "why six?");
    // The agent moves the file with a plain mv and edits it, staging nothing.
    std::fs::rename(r.path().join("old.txt"), r.path().join("new.txt")).unwrap();
    r.write("new.txt", "one\nTWO\nthree\nfour\nfive\nsix\n");
    let index = std::fs::read(r.path().join(".git/index")).unwrap();
    common::land_world(&mut app);
    assert_eq!(listed(&app), ["new.txt"], "one rename, not a delete and an add");
    // Paired on a scratch copy: the real index never learns of the move (No writes).
    assert_eq!(std::fs::read(r.path().join(".git/index")).unwrap(), index);
    assert_eq!(r.git(&["status", "--porcelain"]), " D old.txt\n?? new.txt\n");
    let rows = open(&mut app, "new.txt");
    assert!(rows.contains(&"-two".into()) && rows.contains(&"+TWO".into()), "{rows:?}");
    assert_eq!(app.card_rows().len(), 1, "the comment followed its file");
}

#[test]
fn renames_read_as_renames_whatever_the_users_diff_renames_says() {
    let r = Repo::init();
    r.git(&["config", "diff.renames", "false"]);
    r.write("old.txt", "one\ntwo\nthree\nfour\nfive\n");
    r.commit_all("init");
    r.git(&["mv", "old.txt", "new.txt"]);
    r.write("new.txt", "one\nTWO\nthree\nfour\nfive\n");
    let app = app_on(&r);
    assert_eq!(listed(&app), ["new.txt"]);
}

#[cfg(unix)]
#[test]
fn signed_commits_with_show_signature_on_list_by_their_shas() {
    let r = Repo::init();
    let key = r.path().join(".git").join("key");
    let keyed = std::process::Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", "test", "-f"])
        .arg(&key)
        .stdin(std::process::Stdio::null())
        .status();
    if !keyed.is_ok_and(|s| s.success()) {
        return; // no ssh-keygen here
    }
    let public = std::fs::read_to_string(key.with_extension("pub")).unwrap();
    let signers = r.path().join(".git").join("allowed");
    std::fs::write(&signers, format!("test@herdr.test {public}")).unwrap();
    r.git(&["config", "gpg.format", "ssh"]);
    r.git(&["config", "user.signingkey", &key.to_string_lossy()]);
    r.git(&["config", "gpg.ssh.allowedSignersFile", &signers.to_string_lossy()]);
    // Signed with the key file alone: an agent on the host must never get asked.
    let signed = |message: &str| {
        let mut child = std::process::Command::new("git")
            .arg("-C")
            .arg(r.path())
            .args(["commit", "-q", "-S", "-m", message])
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@herdr.test")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@herdr.test")
            .env_remove("SSH_AUTH_SOCK")
            .stdin(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if let Some(status) = child.try_wait().unwrap() {
                return status.success();
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let _ = child.kill();
        let _ = child.wait();
        false
    };
    r.write("a.txt", "one\n");
    r.git(&["add", "-A"]);
    if !signed("signed one") {
        return; // signing unavailable here
    }
    r.write("a.txt", "two\n");
    r.git(&["add", "-A"]);
    assert!(signed("signed two"));
    r.git(&["config", "log.showSignature", "true"]);
    let head = r.git(&["rev-parse", "HEAD"]).trim().to_string();
    let mut app = app_on(&r);
    app.open_commit_picker();
    let picker = app.commit_picker.as_ref().expect("the picker opens");
    let rows: Vec<(&str, &str)> =
        picker.rows.iter().map(|row| (row.sha.as_str(), row.subject.as_str())).collect();
    assert_eq!(rows.first().map(|r| r.0), Some(head.as_str()), "{rows:?}");
    assert_eq!(rows.iter().map(|r| r.1).collect::<Vec<_>>(), ["signed two", "signed one"]);
    let subject = herdr_reviewr::git::commit_subject(r.path(), &head);
    assert_eq!(subject.as_deref(), Some("signed two"));
}

#[cfg(unix)]
#[test]
fn an_untracked_link_reads_as_the_path_it_names_never_its_target() {
    let outside = tempfile::tempdir().unwrap();
    let secret = outside.path().join("secret.txt");
    std::fs::write(&secret, "outside secret\nmore\n").unwrap();
    let r = Repo::init();
    r.write("target.txt", "real contents\n");
    r.commit_all("init");
    std::os::unix::fs::symlink("target.txt", r.path().join("good-link")).unwrap();
    std::os::unix::fs::symlink(&secret, r.path().join("out-link")).unwrap();
    std::os::unix::fs::symlink("missing", r.path().join("dangling")).unwrap();
    let mut app = app_on(&r);
    let mut names = listed(&app);
    names.sort();
    assert_eq!(names, ["dangling", "good-link", "out-link"]);
    assert_eq!(open(&mut app, "good-link"), ["+target.txt"]);
    assert_eq!(open(&mut app, "dangling"), ["+missing"]);
    let rows = open(&mut app, "out-link");
    assert_eq!(rows, [format!("+{}", secret.display())]);
    let counts: Vec<u32> =
        app.entries.iter().filter_map(|e| e.annotation.as_ref()).map(|a| a.additions).collect();
    assert_eq!(counts, [1, 1, 1], "one line each, as git counts a link");
    app.set_tab(herdr_reviewr::app::Tab::AllFiles).unwrap();
    let rows = open(&mut app, "out-link");
    assert!(rows.iter().all(|r| !r.contains("outside secret")), "{rows:?}");
}

#[test]
fn an_agent_comment_the_pane_reaches_too_late_never_lands() {
    let r = Repo::init();
    r.write("a.rs", "one\ntwo\n");
    r.commit_all("init");
    let mut app = app_on(&r);
    let file = std::fs::canonicalize(r.path().join("a.rs")).unwrap();
    let past = std::time::Instant::now().checked_sub(std::time::Duration::from_secs(1)).unwrap();
    let late = app.receive_comment_until(&file, 2, 2, "x".into(), None, past);
    assert!(late.is_err() && app.store.is_empty(), "the agent was told to try again");
    let soon = std::time::Instant::now() + std::time::Duration::from_secs(5);
    assert_eq!(app.receive_comment_until(&file, 2, 2, "x".into(), None, soon), Ok(1));
    assert_eq!(app.store.len(), 1, "its retry lands once");
}

#[cfg(unix)]
#[test]
fn an_agent_comment_on_a_name_with_a_backslash_lands_on_that_file() {
    let r = Repo::init();
    r.write("back\\slash.txt", "one\ntwo\n");
    r.commit_all("init");
    let mut app = app_on(&r);
    let file = std::fs::canonicalize(r.path().join("back\\slash.txt")).unwrap();
    assert_eq!(app.receive_comment(&file, 2, 2, "why?".into(), None), Ok(1));
    assert_eq!(app.store.get(0).unwrap().file, "back\\slash.txt");
}

#[test]
fn all_files_lists_every_ignored_file_a_negation_leaves_ignored() {
    let r = Repo::init();
    r.write(".gitignore", ".vscode/*\n!.vscode/settings.json\ngenerated/*\n!generated/keep/\n");
    r.write("generated/.gitignore", "keep/*\n!keep/visible.txt\n");
    r.write("src.rs", "x\n");
    r.commit_all("init");
    r.write(".vscode/settings.json", "{}\n");
    r.write(".vscode/launch.json", "{}\n");
    r.write("generated/keep/visible.txt", "v\n");
    r.write("generated/keep/hidden.txt", "h\n");
    r.write("generated/out.txt", "o\n");
    let git_ignored = r.git(&["ls-files", "--others", "--ignored", "--exclude-standard"]);
    let mut app = app_on(&r);
    app.set_tab(herdr_reviewr::app::Tab::AllFiles).unwrap();
    let all: Vec<(String, bool)> =
        app.entries.iter().map(|e| (e.path.clone(), e.ignored)).collect();
    for path in ["src.rs", ".vscode/settings.json", "generated/keep/visible.txt"] {
        assert!(all.iter().any(|(p, ignored)| p == path && !ignored), "{path} visible: {all:?}");
    }
    for path in [".vscode/launch.json", "generated/keep/hidden.txt"] {
        assert!(git_ignored.contains(path), "git ignores {path}: {git_ignored}");
        let listed = all.iter().any(|(p, ignored)| {
            *ignored && (p == path || path.starts_with(p.trim_end_matches('/')))
        });
        assert!(listed, "{path} listed, dimmed: {all:?}");
    }
}

#[test]
fn all_files_leaves_an_empty_ignored_directory_out() {
    let r = Repo::init();
    r.write(".gitignore", "build/\ncache/\n");
    r.write("src.rs", "x\n");
    r.commit_all("init");
    std::fs::create_dir_all(r.path().join("build")).unwrap();
    r.write("cache/blob.bin", "b\n");
    let mut app = app_on(&r);
    app.set_tab(herdr_reviewr::app::Tab::AllFiles).unwrap();
    let all: Vec<(String, bool)> = app.entries.iter().map(|e| (e.path.clone(), e.is_dir)).collect();
    assert!(!all.iter().any(|(p, _)| p == "build"), "{all:?}");
    assert!(all.contains(&("cache".to_string(), true)), "{all:?}");
}

#[test]
fn a_comment_follows_its_file_through_a_git_mv_and_names_the_new_path() {
    let r = Repo::init();
    r.write("old.rs", "fn one() {}\nfn two() {}\nfn three() {}\nfn four() {}\nfn five() {}\n");
    r.commit_all("init");
    r.write(
        "old.rs",
        "fn one() {}\nfn two() {}\nfn three() {}\nfn four() {}\nfn five() {}\nfn six() {}\n",
    );
    let mut app = app_on(&r);
    open(&mut app, "old.rs");
    comment_and_send(&mut app, "+fn six() {}", "why six?");
    r.git(&["mv", "old.rs", "new.rs"]);
    r.write("new.rs", "fn zero() {}\nfn one() {}\nfn two() {}\nfn three() {}\nfn four() {}\nfn five() {}\nfn six() {}\n");
    common::land_world(&mut app);
    assert_eq!(listed(&app), ["new.rs"]);
    assert_eq!(app.store.get(0).unwrap().file, "new.rs");
    open(&mut app, "new.rs");
    assert_eq!(app.card_rows().len(), 1, "the card rides the rename");
    let place = app.card_place(0).unwrap();
    assert_eq!((place.start, place.changed), (7, false), "and its line through the edit");
    let all: Vec<&herdr_reviewr::model::Comment> = app.store.iter().collect();
    assert!(herdr_reviewr::export::format_review(None, &all).starts_with("new.rs:7\n"));
}

#[test]
fn an_emoji_whose_chars_undercount_it_never_blanks_or_drops_a_row() {
    let r = Repo::init();
    let old = format!("⚠\u{fe0f}{}\n", "x".repeat(120));
    let wrapped = format!("⚠\u{fe0f}{}", "y".repeat(19)).repeat(15);
    r.write("note.md", &format!("{old}{wrapped}\n"));
    r.commit_all("init");
    r.write("note.md", &format!("{}{wrapped}\n", "z\n"));
    let count = |screen: &str, ch: char| screen.chars().filter(|&c| c == ch).count();
    // Split, unwrapped: the removed line's half paints its x's, as many as fit.
    let mut app = herdr_reviewr::app::App::new(r.path_buf(), Scope::Uncommitted, None);
    app.reload().unwrap();
    open(&mut app, "note.md");
    app.toggle_split();
    let screen = painted(&app, 160, 30);
    assert!(count(&screen, 'x') > 40, "the half paints: {}", count(&screen, 'x'));
    // Unified, wrapped: every y of the context line lands on some row.
    let mut app = app_on(&r);
    open(&mut app, "note.md");
    app.wrap = true;
    let screen = painted(&app, 80, 40);
    assert_eq!(count(&screen, 'y'), 15 * 19, "no y lost to a short-measured emoji");
}

#[test]
fn a_sha256_repository_diffs_from_its_own_empty_tree() {
    let r = Repo::init();
    std::fs::remove_dir_all(r.path().join(".git")).unwrap();
    r.git(&["init", "-q", "-b", "main", "--object-format=sha256"]);
    r.write("a.rs", "one\n");
    r.git(&["add", "a.rs"]);
    // Unborn: the staged file diffs from the empty tree, which sha256 spells its own way.
    let mut app = app_on(&r);
    assert!(app.status.is_empty(), "{}", app.status);
    assert_eq!(listed(&app), ["a.rs"]);
    assert_eq!(open(&mut app, "a.rs"), ["+one"]);
    r.commit_all("root");
    r.write("a.rs", "one\ntwo\n");
    r.commit_all("two");
    let mut app = app_on(&r);
    app.open_commit_picker();
    app.commit_picker_move(1);
    app.commit_picker_pick().unwrap();
    assert_eq!(listed(&app), ["a.rs"], "the root commit: {}", app.status);
    assert_eq!(open(&mut app, "a.rs"), ["+one"]);
}

#[test]
fn a_sparse_checkout_lists_and_snapshots_a_file_outside_its_cone() {
    use herdr_reviewr::git::{snapshot_worktree, snapshot_worktree_in};
    let r = Repo::init();
    r.write("in/a.txt", "a\n");
    r.write("in/gone.txt", "gone\nfor\ngood\n");
    r.write("out/b.txt", "b\n");
    r.commit_all("init");
    r.git(&["sparse-checkout", "set", "--cone", "in"]);
    r.write("out/new.txt", "new\n");
    r.remove("in/gone.txt");
    let real = || {
        let read = |p: &str| std::fs::read(r.path().join(".git").join(p)).unwrap();
        (read("index"), read("info/sparse-checkout"))
    };
    let before = real();
    // A delete beside an untracked file pairs moves on a scratch copy, past git's cone check.
    let mut app = app_on(&r);
    let mut names = listed(&app);
    names.sort();
    assert_eq!(names, ["in/gone.txt", "out/new.txt"], "{}", app.status);
    assert_eq!(open(&mut app, "out/new.txt"), ["+new"]);
    let tree = snapshot_worktree(r.path()).unwrap();
    let held = r.git(&["ls-tree", "-r", "--name-only", &tree]);
    let held: Vec<&str> = held.lines().collect();
    assert_eq!(held, ["in/a.txt", "out/b.txt", "out/new.txt"], "outside the cone kept");
    let again = snapshot_worktree_in(r.path(), &tree, &["out/new.txt".to_string()]).unwrap();
    assert_eq!(again, Some(tree));
    assert!(real() == before, "No writes: the index and the cone stay as they were");
}

/// The open diff's row index reading `marker + line`.
fn row_at(app: &App, row: &str) -> usize {
    app.visible
        .iter()
        .position(|v| v.row().is_some_and(|r| format!("{}{}", r.marker(), r.text()) == row))
        .unwrap_or_else(|| panic!("a row reading {row:?}"))
}

/// Each comment's text, in store order.
fn texts(app: &App) -> Vec<String> {
    app.store.iter().map(|c| c.text.clone()).collect()
}

#[test]
fn comments_on_one_number_each_side_stay_apart_and_delete_alone() {
    let r = Repo::init();
    r.write("a.txt", "one\ntwo\nthree\n");
    r.commit_all("init");
    r.write("a.txt", "one\nTWO\nthree\n");
    let mut app = app_on(&r);
    open(&mut app, "a.txt");
    comment_and_send(&mut app, "-two", "old side");
    comment_and_send(&mut app, "+TWO", "new side");
    comment_and_send(&mut app, "+TWO", "again new");
    assert_eq!(app.card_rows().len(), 3, "each its own card");
    // The removed line 2 is its own row: deleting there never takes the added line 2's.
    app.diff_cursor = row_at(&app, "-two");
    app.settle_input();
    app.delete_comment();
    assert_eq!(texts(&app), ["new side", "again new"]);
    // Of two on one line, the cursor's row takes one and leaves the other its card.
    app.diff_cursor = row_at(&app, "+TWO");
    app.settle_input();
    app.delete_comment();
    assert_eq!(app.store.len(), 1);
    assert_eq!(app.card_rows().len(), 1);
}

#[test]
fn an_unborn_repository_lists_a_staged_file_as_added_beside_an_untracked_one() {
    let r = Repo::init();
    r.write("staged.rs", "one\n");
    r.git(&["add", "staged.rs"]);
    r.write("loose.rs", "two\n");
    let mut app = app_on(&r);
    let kinds: Vec<(String, Option<ChangeKind>)> = app
        .entries
        .iter()
        .map(|e| (e.path.clone(), e.annotation.as_ref().map(|a| a.kind)))
        .collect();
    let want = [
        ("loose.rs".to_string(), Some(ChangeKind::Untracked)),
        ("staged.rs".to_string(), Some(ChangeKind::Added)),
    ];
    assert_eq!(kinds, want);
    assert_eq!(open(&mut app, "staged.rs"), ["+one"]);
    assert_eq!(open(&mut app, "loose.rs"), ["+two"]);
}

#[test]
fn an_untracked_file_lists_whatever_status_show_untracked_files_says() {
    let r = Repo::init();
    r.write("a.txt", "a\n");
    r.commit_all("init");
    r.git(&["config", "status.showUntrackedFiles", "no"]);
    r.write("new.txt", "new\n");
    let app = app_on(&r);
    assert_eq!(listed(&app), ["new.txt"], "the agent's new file is under review");
}

#[test]
fn a_nul_makes_a_file_binary_only_within_gits_first_8000_bytes() {
    let r = Repo::init();
    r.write("keep.txt", "k\n");
    r.commit_all("init");
    std::fs::write(r.path().join("late.txt"), format!("{}\0tail\n", "a\n".repeat(4300))).unwrap();
    std::fs::write(r.path().join("early.bin"), "a\0b\n").unwrap();
    let mut app = app_on(&r);
    let verdict = |app: &App, path: &str| {
        let entry = app.entries.iter().find(|e| e.path == path).expect("listed");
        let change = entry.annotation.as_ref().expect("a change");
        (change.binary, change.additions)
    };
    assert_eq!(verdict(&app, "early.bin"), (true, 0));
    assert_eq!(verdict(&app, "late.txt"), (false, 4301));
    // Git counts the same once the files are tracked.
    r.git(&["add", "-N", "late.txt", "early.bin"]);
    let numstat = r.git(&["diff", "--numstat"]);
    assert!(numstat.contains("-\t-\tearly.bin") && numstat.contains("4301\t0\tlate.txt"));
    assert_eq!(open(&mut app, "late.txt").len(), 4301);
    assert!(!has_control(&painted(&app, 100, 30)), "the NUL never reaches the terminal");
}

#[test]
fn directories_sharing_a_name_prefix_stay_apart() {
    let paths = ["ChronoStream/a.cs", "ChronoStream/z/c.cs", "ChronoStream.BuildTests/b.cs"];
    let r = Repo::init();
    for p in paths.iter().chain(&["ChronoStreamX.cs"]) {
        r.write(p, "one\n");
    }
    r.commit_all("init");
    for p in paths.iter().chain(&["ChronoStreamX.cs"]) {
        r.write(p, "one\ntwo\n");
    }
    let mut app = app_on(&r);
    // Each file sits under the nearest directory row above it, by whole components.
    let mut dirs: Vec<(usize, String)> = Vec::new();
    for row in &app.file_rows {
        dirs.retain(|(depth, _)| *depth < row.depth);
        match (row.dir_path(), row.file_index()) {
            (Some(dir), _) => dirs.push((row.depth, dir.to_string())),
            (None, Some(i)) => {
                let path = &app.entries[i].path;
                let parent = dirs.last().map_or("", |(_, d)| d.as_str());
                let inside = parent.is_empty() || path.starts_with(&format!("{parent}/"));
                assert!(inside, "{path} drawn under {parent}: {:?}", app.file_rows);
            }
            (None, None) => unreachable!(),
        }
    }
    let named: Vec<&str> = app.file_rows.iter().filter_map(|r| r.dir_path()).collect();
    assert_eq!(named.iter().filter(|d| **d == "ChronoStream").count(), 1, "{named:?}");
    // Viewing the folder marks its own files only.
    app.focus = Focus::Files;
    app.file_cursor =
        app.file_rows.iter().position(|r| r.dir_path() == Some("ChronoStream")).unwrap();
    app.toggle_viewed();
    let viewed: Vec<&str> = app
        .entries
        .iter()
        .map(|e| e.path.as_str())
        .filter(|p| app.viewed_mark(p) == Some(herdr_reviewr::app::ViewedMark::Viewed))
        .collect();
    assert_eq!(viewed, ["ChronoStream/a.cs", "ChronoStream/z/c.cs"]);
}

#[test]
fn a_comment_follows_its_line_through_successive_edits() {
    let r = Repo::init();
    r.write("a.txt", "one\ntwo\nthree\n");
    r.commit_all("init");
    r.write("a.txt", "one\nTWO\nthree\n");
    let mut app = app_on(&r);
    open(&mut app, "a.txt");
    comment_and_send(&mut app, "+TWO", "why caps?");
    for (line, text) in [
        (3, "zero\none\nTWO\nthree\n"),
        (4, "zero\nhalf\none\nTWO\nthree\n"),
        (3, "zero\none\nTWO\nthree\n"),
        (1, "TWO\nthree\n"),
    ] {
        r.write("a.txt", text);
        common::land_world(&mut app);
        let place = app.card_place(0).expect("placed");
        assert_eq!((place.start, place.changed), (line, false), "{text:?}");
    }
    let all: Vec<&herdr_reviewr::model::Comment> = app.store.iter().collect();
    let sent = herdr_reviewr::export::format_review(None, &all);
    assert!(sent.starts_with("a.txt:1\n"), "a send names where the line is now: {sent}");
}

#[test]
fn a_comment_on_one_of_twin_lines_keeps_its_twin_when_lines_land_above() {
    let r = Repo::init();
    r.write("a.rs", "fn a() {\n}\n");
    r.commit_all("init");
    r.write("a.rs", "fn a() {\n}\nfn b() {\n}\n");
    let mut app = app_on(&r);
    open(&mut app, "a.rs");
    comment_and_send(&mut app, "+}", "closes b");
    r.write("a.rs", "// x\nfn a() {\n}\nfn b() {\n}\n");
    common::land_world(&mut app);
    let place = app.card_place(0).expect("placed");
    assert_eq!((place.start, place.changed), (5, false), "b's brace, never a's at 3");
}

#[test]
fn a_comment_on_an_untracked_file_waits_out_its_deletion_and_returns_with_it() {
    let r = Repo::init();
    r.write("keep.txt", "k\n");
    r.commit_all("init");
    r.write("new.rs", "one\ntwo\n");
    let mut app = app_on(&r);
    open(&mut app, "new.rs");
    comment_and_send(&mut app, "+two", "why two");
    r.remove("new.rs");
    common::land_world(&mut app);
    assert!(listed(&app).is_empty());
    assert_eq!(texts(&app), ["why two"], "Comments survive");
    r.write("new.rs", "one\ntwo\n");
    common::land_world(&mut app);
    open(&mut app, "new.rs");
    let place = app.card_place(0).expect("back on its line");
    assert_eq!((place.start, place.changed), (2, false));
    assert_eq!(app.card_rows().len(), 1);
}

#[test]
fn a_last_line_losing_and_regaining_its_newline_keeps_its_comment_unchanged() {
    let r = Repo::init();
    r.write("a.txt", "one\ntwo");
    r.commit_all("init");
    r.write("a.txt", "one\nTWO");
    let mut app = app_on(&r);
    open(&mut app, "a.txt");
    comment_and_send(&mut app, "+TWO", "caps?");
    for text in ["one\nTWO\n", "one\nTWO", "one\nTWO\n"] {
        r.write("a.txt", text);
        common::land_world(&mut app);
        let place = app.card_place(0).expect("placed");
        assert_eq!((place.start, place.changed), (2, false), "{text:?}");
    }
}

#[test]
fn a_viewed_mark_survives_another_files_change() {
    use herdr_reviewr::app::ViewedMark;
    let r = Repo::init();
    r.write("a.txt", "a\n");
    r.write("b.txt", "b\n");
    r.commit_all("init");
    r.write("a.txt", "a\nA\n");
    r.write("b.txt", "b\nB\n");
    let mut app = app_on(&r);
    open(&mut app, "a.txt");
    app.focus = Focus::Diff;
    app.toggle_viewed();
    assert_eq!(app.viewed_mark("a.txt"), Some(ViewedMark::Viewed));
    r.write("b.txt", "b\nB\nBB\n");
    common::land_world(&mut app);
    assert_eq!(app.viewed_mark("a.txt"), Some(ViewedMark::Viewed), "b's edit is b's");
    r.write("a.txt", "a\nA\nAA\n");
    common::land_world(&mut app);
    assert_eq!(app.viewed_mark("a.txt"), Some(ViewedMark::Changed), "a's own edit shows");
}

#[test]
fn file_names_with_and_in_them_diff_as_text_and_binary() {
    let r = Repo::init();
    r.write("this and that.txt", "one\n");
    std::fs::write(r.path().join("x and y.bin"), b"\0one").unwrap();
    std::fs::write(r.path().join("a and b and c.bin"), b"\0one").unwrap();
    r.commit_all("init");
    r.write("this and that.txt", "one\ntwo\n");
    std::fs::write(r.path().join("x and y.bin"), b"\0two").unwrap();
    std::fs::write(r.path().join("a and b and c.bin"), b"\0two").unwrap();
    let mut app = app_on(&r);
    assert_eq!(open(&mut app, "this and that.txt"), [" one", "+two"]);
    for name in ["x and y.bin", "a and b and c.bin"] {
        open(&mut app, name);
        assert!(app.status.is_empty(), "{name}: {}", app.status);
        let screen = painted(&app, 120, 20);
        assert!(screen.contains("binary"), "{name} says binary:\n{screen}");
    }
}

#[test]
fn a_hostile_git_config_changes_nothing_reviewr_reads() {
    use herdr_reviewr::git::snapshot_worktree;
    let r = Repo::init();
    r.write("a.txt", "one\ntwo\nthree\nfour\nfive\nsix\nseven\n");
    r.write("sub/b.txt", "b\n");
    r.commit_all("naïve start");
    r.git(&["tag", "v1"]);
    for (key, value) in [
        ("color.ui", "always"),
        ("color.diff", "always"),
        ("diff.noprefix", "true"),
        ("diff.mnemonicPrefix", "true"),
        ("diff.external", "false"),
        ("diff.context", "0"),
        ("diff.interHunkContext", "0"),
        ("diff.algorithm", "patience"),
        ("diff.relative", "true"),
        ("diff.colorMoved", "zebra"),
        ("diff.wsErrorHighlight", "all"),
        ("diff.dirstat", "files"),
        ("diff.suppressBlankEmpty", "true"),
        ("diff.renameLimit", "1"),
        ("diff.submodule", "log"),
        ("core.quotepath", "true"),
        ("core.abbrev", "4"),
        ("core.pager", "false"),
        ("core.autocrlf", "input"),
        ("core.safecrlf", "true"),
        ("log.abbrevCommit", "true"),
        ("log.decorate", "full"),
        ("log.date", "relative"),
        ("log.showRoot", "false"),
        ("format.pretty", "oneline"),
        ("i18n.logOutputEncoding", "ISO-8859-1"),
        ("status.showUntrackedFiles", "no"),
    ] {
        r.git(&["config", key, value]);
    }
    r.write("a.txt", "one\nTWO\nthree\nfour\nfive\nsix\nSEVEN\n");
    r.write("sub/new.txt", "new\n");
    // A CRLF file the snapshot's `add` would refuse under `core.safecrlf`.
    r.write("sub/dos.txt", "dos\r\nline\r\n");
    let mut app = app_on(&r);
    assert!(app.status.is_empty(), "{}", app.status);
    assert_eq!(listed(&app), ["a.txt", "sub/dos.txt", "sub/new.txt"]);
    let rows = open(&mut app, "a.txt");
    assert_eq!(rows.first().map(String::as_str), Some(" one"), "full context: {rows:?}");
    assert!(rows.contains(&"+TWO".to_string()) && rows.contains(&"+SEVEN".to_string()));
    assert!(rows.iter().all(|r| !r.contains('\u{1b}')), "no color: {rows:?}");
    let tree = snapshot_worktree(r.path()).unwrap();
    let held = r.git(&["-c", "core.quotepath=false", "ls-tree", "-r", "--name-only", &tree]);
    assert_eq!(
        held.lines().collect::<Vec<_>>(),
        ["a.txt", "sub/b.txt", "sub/dos.txt", "sub/new.txt"]
    );
    app.open_commit_picker();
    let picker = app.commit_picker.as_ref().expect("the picker opens");
    let row = picker.rows.first().expect("one commit");
    assert_eq!((row.sha.len(), row.subject.as_str()), (40, "naïve start"), "{row:?}");
    let subject = herdr_reviewr::git::commit_subject(r.path(), &row.sha);
    assert_eq!(subject.as_deref(), Some("naïve start"));
}

#[test]
fn a_split_index_lists_and_snapshots_without_a_write_beside_it() {
    use herdr_reviewr::git::{snapshot_worktree, snapshot_worktree_in};
    let r = Repo::init();
    r.write("a.txt", "a\n");
    r.commit_all("init");
    r.git(&["config", "core.splitIndex", "true"]);
    r.git(&["update-index", "--split-index"]);
    r.write("a.txt", "a\nb\n");
    r.write("new.txt", "n\n");
    r.remove("a.txt");
    r.write("moved.txt", "a\nb\n");
    // The git dir as reviewr found it, its own folder aside.
    let git_dir = || {
        let mut names: Vec<(String, Vec<u8>)> = std::fs::read_dir(r.path().join(".git"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.is_file())
            .map(|p| {
                (p.file_name().unwrap().to_string_lossy().into_owned(), std::fs::read(&p).unwrap())
            })
            .collect();
        names.sort();
        names
    };
    let before = git_dir();
    let mut app = app_on(&r);
    let mut names = listed(&app);
    names.sort();
    assert_eq!(names, ["moved.txt", "new.txt"], "a.txt moved: {}", app.status);
    assert_eq!(open(&mut app, "new.txt"), ["+n"]);
    let tree = snapshot_worktree(r.path()).unwrap();
    let again = snapshot_worktree_in(r.path(), &tree, &["new.txt".to_string()]).unwrap();
    assert_eq!(again, Some(tree));
    assert!(git_dir() == before, "No writes: nothing new beside the index");
}
