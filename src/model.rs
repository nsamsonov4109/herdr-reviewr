//! The review model: scopes, changed files, and the session's comments, never lost to a refresh.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::sync::Arc;

/// Which set of changes the Changes view shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scope {
    Uncommitted,
    Branch,
    /// What changed after the reviewer's last send, each repository against its snapshot then.
    SinceSend,
    LastTurn,
    /// A picked run of commits, diffed `A^` against `B`.
    Commits,
}

impl Scope {
    pub fn label(self) -> &'static str {
        match self {
            Scope::Uncommitted => "uncommitted",
            Scope::Branch => "branch",
            Scope::SinceSend => "since send",
            Scope::LastTurn => "last turn",
            Scope::Commits => "commits",
        }
    }

    /// The kebab-case name config values use (`default_scope`).
    pub fn name(self) -> &'static str {
        match self {
            Scope::Uncommitted => "uncommitted",
            Scope::Branch => "branch",
            Scope::SinceSend => "since-send",
            Scope::LastTurn => "last-turn",
            Scope::Commits => "commits",
        }
    }

    /// The next scope, for a click on the header chip.
    #[must_use]
    pub fn cycle(self) -> Self {
        match self {
            Scope::Uncommitted => Scope::Branch,
            Scope::Branch => Scope::SinceSend,
            Scope::SinceSend => Scope::LastTurn,
            Scope::LastTurn => Scope::Commits,
            Scope::Commits => Scope::Uncommitted,
        }
    }
}

/// The `commits` pick: the run `oldest..=newest`, full ids, equal for a run of one.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CommitPick {
    pub oldest: String,
    pub newest: String,
}

impl CommitPick {
    pub fn single(sha: &str) -> Self {
        Self { oldest: sha.to_string(), newest: sha.to_string() }
    }

    pub fn is_single(&self) -> bool {
        self.oldest == self.newest
    }

    /// `1a2b3c4`, or `1a2b3c4 to 5d6e7f8` for a run, oldest first: git's `..` would leave it out.
    pub fn label(&self) -> String {
        let short = crate::git::abbreviate_oid;
        if self.is_single() {
            short(&self.newest)
        } else {
            format!("{} to {}", short(&self.oldest), short(&self.newest))
        }
    }
}

/// The `commits` scope's picks by repository name (`""` for a lone repository).
pub type Picks = BTreeMap<String, CommitPick>;

/// Where a comment's diff was read; it renders only while the scope reads that same diff.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Rev {
    Worktree,
    Commit(CommitPick),
}

/// How a file changed within a scope.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    Untracked,
}

impl ChangeKind {
    pub fn marker(self) -> char {
        match self {
            ChangeKind::Added => 'A',
            ChangeKind::Modified => 'M',
            ChangeKind::Deleted => 'D',
            ChangeKind::Renamed => 'R',
            ChangeKind::Copied => 'C',
            ChangeKind::Untracked => '?',
        }
    }
}

/// A row in the Changes list.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ChangedFile {
    pub path: String,
    pub kind: ChangeKind,
    pub additions: u32,
    pub deletions: u32,
    /// The source path of a renamed or copied file, whose old content lives there.
    pub previous_path: Option<String>,
    /// git's no-text-diff verdict, read as the binary notice.
    pub binary: bool,
    /// The bytes git stores on the old side, 0 where there is none.
    pub old_size: u64,
    /// The bytes git stores on the new side, 0 where there is none; `None` for the worktree.
    pub new_size: Option<u64>,
    /// Its git mode at each end when the change moved it, as `100644` to `100755`.
    pub mode: Option<(u32, u32)>,
}

impl ChangedFile {
    /// Whether `paths` cover this file or the source it was renamed from.
    pub fn touched_by<'a>(&self, paths: impl IntoIterator<Item = &'a String> + Clone) -> bool {
        crate::git::covered(paths.clone(), &self.path)
            || self.previous_path.as_deref().is_some_and(|p| crate::git::covered(paths, p))
    }
}

/// Which side of the diff a comment's lines live on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    New,
    Old,
}

/// A reviewer comment anchored to a run of diff lines, carrying the snippet.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Comment {
    pub file: String,
    pub side: Side,
    pub start: u32,
    pub end: u32,
    /// Verbatim diff lines the comment anchors to, each keeping its `+`/`-`/space marker.
    pub lines: String,
    pub text: String,
    /// A diff comment (`Changes`), else a content comment (`All files`).
    pub diff_anchored: bool,
    /// Where the new side was read.
    pub rev: Rev,
}

impl Comment {
    /// `path:start-end` or `path:line`, saying ` (removed)` and ` (in commit 1a2b3c4)` as they apply.
    pub fn location(&self) -> String {
        let file = quoted(&self.file);
        let range = if self.start == self.end {
            format!("{file}:{}", self.start)
        } else {
            format!("{file}:{}-{}", self.start, self.end)
        };
        let commit = match &self.rev {
            Rev::Worktree => None,
            Rev::Commit(pick) if pick.is_single() => Some(format!("commit {}", pick.label())),
            Rev::Commit(pick) => Some(format!("commits {}", pick.label())),
        };
        match (self.side, commit) {
            (Side::New, None) => range,
            (Side::Old, None) => format!("{range} (removed)"),
            (Side::New, Some(commit)) => format!("{range} (in {commit})"),
            (Side::Old, Some(commit)) => format!("{range} (removed in {commit})"),
        }
    }
}

/// `path` as a send names it: quoted as git quotes it when a control character in it would split
/// or garble the header line, else as is.
fn quoted(path: &str) -> std::borrow::Cow<'_, str> {
    if !path.chars().any(char::is_control) {
        return std::borrow::Cow::Borrowed(path);
    }
    let mut out = String::from("\"");
    for c in path.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => {
                let _ = write!(out, "\\{:03o}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    std::borrow::Cow::Owned(out)
}

/// Who wrote one message of a thread.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub enum Author {
    #[default]
    Reviewer,
    /// An agent, by the name the review went to.
    Agent(String),
}

/// One message after a thread's opening comment.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Reply {
    pub author: Author,
    pub text: String,
    /// Whether it reached the agent: a reviewer's reply waits for the next send.
    pub sent: bool,
}

/// The conversation a send opens on a comment: the replies after it, and whether it is closed.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Thread {
    /// Who wrote the comment itself: the reviewer, or an agent pointing at its own code.
    pub opener: Author,
    pub replies: Vec<Reply>,
    pub resolved: bool,
    /// An agent's answer the reviewer has not been taken to yet.
    pub unread: bool,
}

impl Thread {
    /// Whether the agent spoke last: a reply waits for the reviewer.
    pub fn answered(&self) -> bool {
        self.replies.last().is_some_and(|r| matches!(r.author, Author::Agent(_)))
    }
}

/// What a comment's lines read when it was made, and where it last found them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Anchor {
    /// The version of the text its line numbers count in; `None` until one is known.
    version: Option<u64>,
    /// Its lines as they read when it was made: what it follows.
    held: Vec<String>,
    /// The lines under it no longer read as `held`.
    changed: bool,
}

/// The session's comments, each with an id never reused, so a reference never lands on another.
#[derive(Default, Debug)]
pub struct CommentStore {
    items: Vec<Comment>,
    ids: Vec<u64>,
    /// Each comment's thread once a send carried it; `None` while it is a draft.
    threads: Vec<Option<Thread>>,
    anchors: Vec<Anchor>,
    /// The texts comments last placed themselves in, by version, while one still counts in it.
    texts: HashMap<u64, Arc<str>>,
    next_id: u64,
}

impl CommentStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Comment> {
        self.items.iter()
    }

    pub fn get(&self, index: usize) -> Option<&Comment> {
        self.items.get(index)
    }

    /// Append a comment, its lines read from its snippet; returns its index.
    pub fn add(&mut self, comment: Comment) -> usize {
        let held = snippet_side(&comment);
        self.anchors.push(Anchor { version: None, held, changed: false });
        self.items.push(comment);
        self.ids.push(self.next_id);
        self.threads.push(None);
        self.next_id += 1;
        self.items.len() - 1
    }

    /// Append a comment made on `text`, the side its lines count in; returns its index.
    pub fn add_on(&mut self, comment: Comment, text: &str) -> usize {
        let lines = crate::anchor::lines(text);
        let run = (comment.start.max(1) as usize - 1)..(comment.end as usize);
        let held = lines.get(run).map(|run| run.iter().map(|l| (*l).to_string()).collect());
        let i = self.add(comment);
        if let Some(held) = held {
            let version = crate::anchor::version(&lines);
            self.texts.entry(version).or_insert_with(|| text.into());
            self.anchors[i] = Anchor { version: Some(version), held, changed: false };
        }
        i
    }

    /// Append an agent's own comment made on `text`, a thread it opens unread; returns its index.
    pub fn add_from_agent(&mut self, comment: Comment, text: &str, name: String) -> usize {
        let i = self.add_on(comment, text);
        let opener = Author::Agent(name);
        self.threads[i] = Some(Thread { opener, unread: true, ..Thread::default() });
        i
    }

    /// Move the comments `wants` picks on `from` to `to`, a rename's new path; how many moved.
    pub fn rename(&mut self, from: &str, to: &str, wants: impl Fn(&Comment) -> bool) -> usize {
        let mut moved = 0;
        for c in self.items.iter_mut().filter(|c| c.file == from && wants(c)) {
            c.file = to.to_string();
            moved += 1;
        }
        moved
    }

    /// Whether the lines under comment `index` changed since it was made.
    pub fn changed(&self, index: usize) -> bool {
        self.anchors.get(index).is_some_and(|a| a.changed)
    }

    /// The lines comment `index` was made on, as they read then.
    pub fn held(&self, index: usize) -> &[String] {
        self.anchors.get(index).map_or(&[], |a| a.held.as_slice())
    }

    /// The text comment `index`'s line numbers count in, while the store holds it.
    pub fn counted_in(&self, index: usize) -> Option<&str> {
        let version = self.anchors.get(index)?.version?;
        self.texts.get(&version).map(|text| &**text)
    }

    /// Carry the comments `wants` picks into `text`, their side as it reads now; whether any moved.
    pub fn follow(&mut self, wants: impl Fn(&Comment) -> bool, text: &str) -> bool {
        // Most files hold no comment: they cost no read of their text.
        if !self.items.iter().any(&wants) {
            return false;
        }
        let now = crate::anchor::lines(text);
        let version = crate::anchor::version(&now);
        let mut befores: HashMap<u64, Vec<&str>> = HashMap::new();
        let (mut moved, mut placed_any) = (false, false);
        for (c, anchor) in self.items.iter_mut().zip(&mut self.anchors) {
            if !wants(c) || anchor.version == Some(version) {
                continue;
            }
            let before = anchor.version.and_then(|v| {
                let text = self.texts.get(&v)?;
                Some(befores.entry(v).or_insert_with(|| crate::anchor::lines(text)).clone())
            });
            let placed =
                crate::anchor::follow(&anchor.held, c.start, c.end, before.as_deref(), &now);
            moved |= (placed.start, placed.end, placed.changed) != (c.start, c.end, anchor.changed);
            (c.start, c.end, anchor.changed) = (placed.start, placed.end, placed.changed);
            anchor.version = Some(version);
            placed_any = true;
        }
        if placed_any {
            self.texts.entry(version).or_insert_with(|| text.into());
            self.prune_texts();
        }
        moved
    }

    /// Drop every text no comment counts its lines in any more.
    fn prune_texts(&mut self) {
        let used: HashSet<u64> = self.anchors.iter().filter_map(|a| a.version).collect();
        self.texts.retain(|v, _| used.contains(v));
    }

    /// The number a send names the comment at `index` by, from 1, never reused.
    pub fn number(&self, index: usize) -> Option<u64> {
        self.id(index).map(|id| id + 1)
    }

    /// The index of the comment numbered `number`, while the store holds it.
    pub fn index_of_number(&self, number: u64) -> Option<usize> {
        self.index_of(number.checked_sub(1)?)
    }

    /// The thread of the comment at `index`, once a send carried it.
    pub fn thread(&self, index: usize) -> Option<&Thread> {
        self.threads.get(index)?.as_ref()
    }

    /// The numbers of the threads an agent may answer: sent, and not resolved.
    pub fn open_numbers(&self) -> Vec<u64> {
        (0..self.items.len())
            .filter(|&i| self.thread(i).is_some_and(|t| !t.resolved))
            .filter_map(|i| self.number(i))
            .collect()
    }

    /// What the next send carries: every draft, and every reply of the reviewer's not yet sent.
    pub fn unsent(&self) -> usize {
        self.threads
            .iter()
            .map(|t| match t {
                None => 1,
                Some(t) => t.replies.iter().filter(|r| !r.sent).count(),
            })
            .sum()
    }

    /// A send delivered everything unsent: drafts open threads, replies count as sent.
    pub fn mark_sent(&mut self) {
        for thread in &mut self.threads {
            let thread = thread.get_or_insert_with(Thread::default);
            for reply in &mut thread.replies {
                reply.sent = true;
            }
        }
    }

    /// Append `reply` to the thread of the comment at `index`, reopening it; `false` when it has
    /// none. An agent's is unread until the reviewer goes to it; the reviewer's own reads the thread.
    pub fn reply(&mut self, index: usize, reply: Reply) -> bool {
        match self.threads.get_mut(index) {
            Some(Some(thread)) => {
                thread.unread = matches!(reply.author, Author::Agent(_));
                thread.replies.push(reply);
                // A reply is a question still open: the agent's answer must reach it.
                thread.resolved = false;
                true
            }
            _ => false,
        }
    }

    /// The reviewer went to the thread of the comment at `index`: its answers are read.
    pub fn mark_read(&mut self, index: usize) {
        if let Some(Some(thread)) = self.threads.get_mut(index) {
            thread.unread = false;
        }
    }

    /// How many open threads hold an answer the reviewer has not been taken to.
    pub fn unread(&self) -> usize {
        self.threads.iter().flatten().filter(|t| t.unread && !t.resolved).count()
    }

    /// Close or reopen the thread of the comment at `index`; `false` when it has none.
    pub fn set_resolved(&mut self, index: usize, resolved: bool) -> bool {
        match self.threads.get_mut(index) {
            Some(Some(thread)) => {
                thread.resolved = resolved;
                thread.unread &= !resolved;
                true
            }
            _ => false,
        }
    }

    /// The id of the comment at `index`.
    pub fn id(&self, index: usize) -> Option<u64> {
        self.ids.get(index).copied()
    }

    /// The index of the comment with id `id`, while the store holds it.
    pub fn index_of(&self, id: u64) -> Option<usize> {
        self.ids.iter().position(|&i| i == id)
    }

    /// Replace the text of the comment at `index`. Returns `false` if out of range.
    pub fn edit(&mut self, index: usize, text: String) -> bool {
        if let Some(c) = self.items.get_mut(index) {
            c.text = text;
            true
        } else {
            false
        }
    }

    /// Remove and return the comment at `index` (delete, or consume one on export).
    pub fn take(&mut self, index: usize) -> Option<Comment> {
        if index < self.items.len() {
            self.ids.remove(index);
            self.threads.remove(index);
            self.anchors.remove(index);
            self.prune_texts();
            Some(self.items.remove(index))
        } else {
            None
        }
    }

    /// Remove and return every comment (consume-all on a successful export).
    pub fn take_all(&mut self) -> Vec<Comment> {
        self.ids.clear();
        self.threads.clear();
        self.anchors.clear();
        self.texts.clear();
        std::mem::take(&mut self.items)
    }
}

/// The lines of `c`'s snippet on its own side, markers dropped.
fn snippet_side(c: &Comment) -> Vec<String> {
    let ours = |marker: char| match c.side {
        Side::New => marker != '-',
        Side::Old => marker != '+',
    };
    c.lines
        .lines()
        .filter_map(|line| {
            let mut chars = line.chars();
            let marker = chars.next()?;
            ours(marker).then(|| chars.as_str().to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Author, Comment, CommentStore, Reply, Rev, Scope, Side};

    fn comment(file: &str, start: u32, end: u32, text: &str) -> Comment {
        Comment {
            file: file.into(),
            side: Side::New,
            start,
            end,
            lines: "+x".into(),
            text: text.into(),
            diff_anchored: true,
            rev: Rev::Worktree,
        }
    }

    #[test]
    fn scope_cycles_and_labels() {
        // The chip click cycles through all four scopes and wraps.
        assert_eq!(Scope::Uncommitted.cycle(), Scope::Branch);
        assert_eq!(Scope::Branch.cycle(), Scope::SinceSend);
        assert_eq!(Scope::SinceSend.cycle(), Scope::LastTurn);
        assert_eq!(Scope::SinceSend.label(), "since send");
        assert_eq!(Scope::LastTurn.cycle(), Scope::Commits);
        assert_eq!(Scope::Commits.cycle(), Scope::Uncommitted);
        assert_eq!(Scope::Uncommitted.label(), "uncommitted");
        assert_eq!(Scope::LastTurn.label(), "last turn");
        assert_eq!(Scope::Commits.label(), "commits");
        assert_eq!(Scope::Commits.name(), "commits");
    }

    #[test]
    fn a_pick_of_one_is_single() {
        let pick = super::CommitPick::single("abc");
        assert!(pick.is_single());
        assert!(!super::CommitPick { oldest: "a".into(), newest: "b".into() }.is_single());
    }

    #[test]
    fn location_formats_range_single_and_removed() {
        let mut c = comment("a.rs", 40, 52, "x");
        assert_eq!(c.location(), "a.rs:40-52");
        c.end = 40;
        assert_eq!(c.location(), "a.rs:40");
        c.side = Side::Old;
        assert_eq!(c.location(), "a.rs:40 (removed)");
        // A comment made on a commit names it, so the agent reads the lines it saw.
        let (one, two) = ("1a2b3c4d".repeat(5), "5d6e7f80".repeat(5));
        c.rev = Rev::Commit(super::CommitPick::single(&one));
        assert_eq!(c.location(), "a.rs:40 (removed in commit 1a2b3c4)");
        c.side = Side::New;
        c.rev = Rev::Commit(super::CommitPick { oldest: one, newest: two });
        assert_eq!(c.location(), "a.rs:40 (in commits 1a2b3c4 to 5d6e7f8)");
        // A control character in the name would split the header: quoted as git quotes it.
        let mut odd = comment("two\nlines\t\u{1b}x.rs", 3, 3, "x");
        assert_eq!(odd.location(), "\"two\\nlines\\t\\033x.rs\":3");
        odd.file = "back\\slash \"q\".rs".into();
        assert_eq!(odd.location(), "back\\slash \"q\".rs:3", "printable names stay as they are");
    }

    #[test]
    fn add_get_edit() {
        let mut s = CommentStore::new();
        let i = s.add(comment("a.rs", 1, 1, "first"));
        assert_eq!(s.len(), 1);
        assert_eq!(s.get(i).unwrap().text, "first");
        assert!(s.edit(i, "second".into()));
        assert_eq!(s.get(i).unwrap().text, "second");
        assert!(!s.edit(99, "nope".into()));
    }

    #[test]
    fn a_send_opens_threads_and_only_new_words_wait_for_the_next() {
        let mut s = CommentStore::new();
        s.add(comment("a.rs", 1, 1, "first"));
        s.add(comment("b.rs", 2, 2, "second"));
        assert_eq!((s.number(0), s.number(1)), (Some(1), Some(2)), "numbered from 1");
        assert_eq!(s.unsent(), 2, "two drafts");
        assert!(s.thread(0).is_none() && s.open_numbers().is_empty(), "a draft is no thread");
        let reply = |author: Author, sent| Reply { author, text: "r".into(), sent };
        assert!(!s.reply(0, reply(Author::Reviewer, false)), "a draft takes no reply");

        s.mark_sent();
        assert_eq!(s.unsent(), 0);
        assert_eq!(s.open_numbers(), [1, 2]);
        assert!(s.reply(0, reply(Author::Agent("claude".into()), true)));
        assert!(s.thread(0).unwrap().answered(), "the agent spoke last");
        assert!(s.thread(0).unwrap().unread, "the agent's answer waits unread");
        assert!(s.reply(0, reply(Author::Reviewer, false)));
        assert!(!s.thread(0).unwrap().unread, "replying reads the thread");
        assert_eq!(s.unsent(), 1, "the reviewer's reply waits");
        s.mark_sent();
        assert_eq!(s.unsent(), 0);
        assert!(s.thread(0).unwrap().replies.iter().all(|r| r.sent));

        assert!(s.reply(1, reply(Author::Agent("claude".into()), true)));
        assert_eq!(s.unread(), 1);
        s.mark_read(1);
        assert_eq!(s.unread(), 0);
        assert!(s.reply(1, reply(Author::Agent("claude".into()), true)));
        assert!(s.set_resolved(1, true));
        assert_eq!(s.unread(), 0, "a resolved thread holds nothing unread");
        assert_eq!(s.open_numbers(), [1], "a resolved thread takes no answer");
        s.take(0);
        assert_eq!(s.index_of_number(2), Some(0), "a number outlives its index");
        assert_eq!(s.index_of_number(1), None);
        assert_eq!(s.index_of_number(0), None);
    }

    #[test]
    fn take_one_and_take_all_consume() {
        let mut s = CommentStore::new();
        s.add(comment("a.rs", 1, 1, "one"));
        s.add(comment("b.rs", 2, 2, "two"));
        let taken = s.take(0).unwrap();
        assert_eq!(taken.text, "one");
        assert_eq!(s.len(), 1);
        let rest = s.take_all();
        assert_eq!(rest.len(), 1);
        assert!(s.is_empty());
        assert!(s.take(0).is_none());
    }

    #[test]
    fn ids_are_never_reused_and_survive_an_edit() {
        let mut s = CommentStore::new();
        s.add(comment("a.rs", 1, 1, "one"));
        let two = s.add(comment("a.rs", 2, 2, "two"));
        let id = s.id(two).unwrap();
        s.take(0);
        assert_eq!(s.index_of(id), Some(0), "the list shifted, the id followed");
        s.edit(0, "edited".into());
        assert_eq!(s.id(0), Some(id));
        s.take_all();
        let three = s.add(comment("a.rs", 3, 3, "three"));
        assert_ne!(s.id(three), Some(id), "a fresh comment never takes an old id");
        assert_eq!(s.index_of(id), None);
    }
}
