//! The world snapshot: what one refresh derives from git alone, built on the caller or the worker.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::Receiver;

use anyhow::{Context, Result};

use crate::app::Tab;
use crate::file_list::Entry;
use crate::git;
use crate::model::{ChangeKind, ChangedFile, CommitPick, Scope};
use crate::repos::Repos;

/// Everything the build reads; a snapshot lands only while the view still matches it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WorldInput {
    pub repo: PathBuf,
    /// A folder's repositories, each built as its own; empty when `repo` is the one repository.
    pub repos: Repos,
    pub tab: Tab,
    pub scope: Scope,
    /// The `--base` flag. The pick is read at build time, so another pane's pick lands as content.
    pub base: Option<String>,
    /// The config's `default_base`, tried before the default branch.
    pub default_base: Vec<String>,
    /// Bumped by this pane's pick, so a build of the previous pick never lands.
    pub base_epoch: u64,
    /// The `last-turn` baseline tree the changed set diffs against; `None` before a turn.
    pub turn_baseline: Option<String>,
    /// The trees the repositories held at the reviewer's last send, by name (`""` for a lone
    /// repository): what `since send` diffs against.
    pub send_baselines: BTreeMap<String, String>,
    /// The `commits` scope's pick, so a build of a replaced pick never lands.
    pub commit_pick: Option<CommitPick>,
    /// Expanded ignored directories whose children the `All files` tree loads.
    pub toggled_dirs: HashSet<String>,
}

impl WorldInput {
    /// The input of the folder's repository `name`, its expanded directories spelled inside it.
    fn of_repo(&self, name: &str) -> Self {
        let inside = format!("{name}/");
        let toggled = self.toggled_dirs.iter().filter_map(|d| d.strip_prefix(&inside));
        Self {
            repo: self.repo.join(name),
            repos: Repos::default(),
            tab: self.tab,
            scope: self.scope,
            base: self.base.clone(),
            default_base: self.default_base.clone(),
            base_epoch: self.base_epoch,
            turn_baseline: self.turn_baseline.clone(),
            send_baselines: self
                .send_baselines
                .get(name)
                .map_or_else(BTreeMap::new, |tree| BTreeMap::from([(String::new(), tree.clone())])),
            commit_pick: self.commit_pick.clone(),
            toggled_dirs: toggled.map(str::to_string).collect(),
        }
    }
}

/// One refresh's result; the base rides along so the header and its changeset land together.
#[derive(Clone, Debug)]
pub struct WorldSnapshot {
    pub changeset: Changeset,
    pub entries: Vec<Entry>,
    pub branch_base: git::BaseStatus,
    /// The `commits` scope's pick verdict; `None` on every other scope.
    pub pick_status: Option<PickStatus>,
    /// `HEAD` at build time, the commit picker's key; `None` when unborn.
    pub head: Option<String>,
    /// The paths a path-limited build re-read; `None` for a full build, which re-read everything.
    pub touched: Option<BTreeSet<String>>,
    /// A folder's `branch` bases by repository; `branch_base` then stays empty.
    pub repo_bases: BTreeMap<String, git::BaseStatus>,
    /// The folder's repositories whose build failed, with why; the worker shows its last build of each.
    pub failed: Vec<(String, String)>,
}

/// What one build found the commit pick to be.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PickVerdict {
    /// Every commit reachable from `HEAD`.
    Live,
    /// Some commit unreachable from `HEAD`. The run still paints.
    OffBranch,
    /// A needed commit is pruned, named here. The scope is empty.
    Gone(String),
}

/// The pick's verdict and the newest commit's subject, for the header.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PickStatus {
    pub verdict: PickVerdict,
    pub subject: String,
    /// How many commits the run spans, `0` when `gone` or not a run.
    pub count: usize,
}

/// The ends a changeset was diffed between (`new` `None` for the worktree); a file's diff reads these.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffEnds {
    pub old: String,
    pub new: Option<String>,
}

/// A scope's changed files by path and the ends they were diffed between, landed together.
#[derive(Clone, Debug, Default)]
pub struct Changeset {
    pub files: BTreeMap<String, ChangedFile>,
    /// `None` when the scope has nothing to diff: no baseline, base, or live pick.
    pub ends: Option<DiffEnds>,
    /// A folder's ends by repository, one with nothing to diff absent; `ends` then stays `None`.
    pub repo_ends: BTreeMap<String, DiffEnds>,
}

impl Changeset {
    /// The repository `path`'s diff reads in, the path inside it, and the ends it reads.
    pub fn ends_of<'s, 'r, 'a>(
        &'s self,
        repos: &'r Repos,
        path: &'a str,
    ) -> Option<(Option<&'r str>, &'a str, &'s DiffEnds)> {
        if !repos.is_folder() {
            return self.ends.as_ref().map(|ends| (None, path, ends));
        }
        let (name, inner) = repos.locate(path)?;
        self.repo_ends.get(name).map(|ends| (Some(name), inner, ends))
    }
}

/// A build's changeset and the base or pick it diffs against, landed together.
#[derive(Debug, Default)]
pub struct ScopeBuild {
    pub branch_base: git::BaseStatus,
    pub pick_status: Option<PickStatus>,
    pub changeset: Changeset,
    /// A folder's `branch` bases by repository.
    pub repo_bases: BTreeMap<String, git::BaseStatus>,
}

/// Build the snapshot for `input`; the changeset is built on every tab.
pub fn build(input: &WorldInput) -> Result<WorldSnapshot> {
    if input.repos.is_folder() {
        let names = input.repos.names();
        let built = crate::repos::par_map(names, |name| build(&input.of_repo(name)));
        let mut parts = BTreeMap::new();
        let mut failed = Vec::new();
        for (name, built) in names.iter().zip(built) {
            match built {
                Ok(snapshot) => drop(parts.insert(name.clone(), snapshot)),
                Err(e) => failed.push((name.clone(), format!("{e:#}"))),
            }
        }
        return Ok(compose(&parts, None, failed));
    }
    // Outside a repo, paint the quiet empty state, not an error every refresh.
    if !git::is_repo(&input.repo) {
        return Ok(WorldSnapshot {
            changeset: Changeset::default(),
            entries: Vec::new(),
            branch_base: git::BaseStatus::default(),
            pick_status: None,
            head: None,
            touched: None,
            repo_bases: BTreeMap::new(),
            failed: Vec::new(),
        });
    }
    // One read of HEAD serves the snapshot and the uncommitted diff's old end.
    let head = git::head_oid(&input.repo);
    let ScopeBuild { branch_base, pick_status, changeset, .. } = scope_build(input, head.clone())?;
    let entries = match input.tab {
        // The whole worktree (ignored included), with expanded ignored dirs loaded lazily.
        Tab::AllFiles => all_files_entries(input, &changeset.files)?,
        // `Changes` (the `PR` tab never builds a snapshot).
        _ => changeset.files.values().map(Entry::from_changed).collect(),
    };
    Ok(WorldSnapshot {
        changeset,
        entries,
        branch_base,
        pick_status,
        head,
        touched: None,
        repo_bases: BTreeMap::new(),
        failed: Vec::new(),
    })
}

/// Every repository's worktree as a tree, by name (`""` for a lone repository), as a send's
/// round starts from it; with the repositories it failed in, and why.
pub fn snapshot_all(
    root: &Path,
    repos: &Repos,
) -> (BTreeMap<String, String>, Vec<(String, String)>) {
    let names: Vec<String> =
        if repos.is_folder() { repos.names().to_vec() } else { vec![String::new()] };
    let trees = crate::repos::par_map(&names, |name| git::snapshot_worktree(&root.join(name)));
    let mut baselines = BTreeMap::new();
    let mut failed = Vec::new();
    for (name, tree) in names.into_iter().zip(trees) {
        match tree {
            Ok(tree) => drop(baselines.insert(name, tree)),
            Err(e) => failed.push((name, format!("{e:#}"))),
        }
    }
    (baselines, failed)
}

/// Merge a folder's repository builds into one, every path spelled from the folder.
fn compose(
    parts: &BTreeMap<String, WorldSnapshot>,
    touched: Option<BTreeSet<String>>,
    failed: Vec<(String, String)>,
) -> WorldSnapshot {
    let mut changeset = Changeset::default();
    let mut entries = Vec::new();
    let mut repo_bases = BTreeMap::new();
    for (name, part) in parts {
        let file = |f: &ChangedFile| ChangedFile {
            path: crate::repos::join(name, &f.path),
            previous_path: f.previous_path.as_deref().map(|p| crate::repos::join(name, p)),
            ..f.clone()
        };
        changeset.files.extend(part.changeset.files.values().map(|f| {
            let f = file(f);
            (f.path.clone(), f)
        }));
        if let Some(ends) = &part.changeset.ends {
            changeset.repo_ends.insert(name.clone(), ends.clone());
        }
        entries.extend(part.entries.iter().map(|e| Entry {
            path: crate::repos::join(name, &e.path),
            annotation: e.annotation.as_ref().map(file),
            ..e.clone()
        }));
        repo_bases.insert(name.clone(), part.branch_base.clone());
    }
    // `a-b/` sorts before `a/`, so the repositories' sorted lists need one more sort.
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    WorldSnapshot {
        changeset,
        entries,
        branch_base: git::BaseStatus::default(),
        pick_status: None,
        head: None,
        touched,
        repo_bases,
        failed,
    }
}

/// The active scope's changeset and, on `branch`, its base.
pub fn build_changed(input: &WorldInput) -> Result<ScopeBuild> {
    if input.repos.is_folder() {
        let names = input.repos.names();
        let parts = crate::repos::par_map(names, |name| build_changed(&input.of_repo(name)));
        let mut build = ScopeBuild::default();
        for (name, part) in names.iter().zip(parts) {
            let part = part.with_context(|| name.clone())?;
            for f in part.changeset.files.into_values() {
                let path = crate::repos::join(name, &f.path);
                let previous_path = f.previous_path.as_deref().map(|p| crate::repos::join(name, p));
                build
                    .changeset
                    .files
                    .insert(path.clone(), ChangedFile { path, previous_path, ..f });
            }
            if let Some(ends) = part.changeset.ends {
                build.changeset.repo_ends.insert(name.clone(), ends);
            }
            build.repo_bases.insert(name.clone(), part.branch_base);
        }
        return Ok(build);
    }
    if !git::is_repo(&input.repo) {
        return Ok(ScopeBuild::default());
    }
    scope_build(input, git::head_oid(&input.repo))
}

/// [`build_changed`] against `head`, as the caller read it, inside a repo.
fn scope_build(input: &WorldInput, head: Option<String>) -> Result<ScopeBuild> {
    match input.scope {
        Scope::LastTurn => match input.turn_baseline.as_deref() {
            Some(t) => {
                let now = git::snapshot_worktree(&input.repo)?;
                at_ends(&input.repo, DiffEnds { old: t.to_string(), new: Some(now) })
            }
            None => Ok(ScopeBuild::default()),
        },
        Scope::SinceSend => match input.send_baselines.get("") {
            Some(sent) => {
                let now = git::snapshot_worktree(&input.repo)?;
                at_ends(&input.repo, DiffEnds { old: sent.clone(), new: Some(now) })
            }
            None => Ok(ScopeBuild::default()),
        },
        Scope::Uncommitted => {
            let base = git::diff_base(head);
            at_ends(&input.repo, DiffEnds { old: base, new: None })
        }
        Scope::Branch => {
            // A resolve failure fails the build, keeping the stale frame.
            let resolution =
                git::resolve_base_with(&input.repo, input.base.as_deref(), &input.default_base)?;
            let merge_base = resolution
                .status
                .winner
                .as_ref()
                .and_then(|w| git::merge_base(&input.repo, w.oid()));
            let build = match merge_base {
                Some(base) => at_ends(&input.repo, DiffEnds { old: base, new: None })?,
                None => ScopeBuild::default(),
            };
            Ok(ScopeBuild { branch_base: resolution.status, ..build })
        }
        Scope::Commits => {
            // A tag without a pick builds the empty changeset.
            let Some(pick) = &input.commit_pick else { return Ok(ScopeBuild::default()) };
            build_pick(&input.repo, pick)
        }
    }
}

/// The changeset between `ends`, carried beside them: the ends a file's diff reads are its input.
fn at_ends(repo: &Path, ends: DiffEnds) -> Result<ScopeBuild> {
    let changed = match &ends.new {
        None => git::changed_from(repo, &ends.old)?,
        Some(new) => git::changed_between(repo, &ends.old, new)?,
    };
    let files = changed.into_iter().map(|f| (f.path.clone(), f)).collect();
    let changeset = Changeset { files, ends: Some(ends), ..Changeset::default() };
    Ok(ScopeBuild { changeset, ..ScopeBuild::default() })
}

/// The pick's changeset, verdict and ends in one pass; a `gone` pick has neither.
fn build_pick(repo: &Path, pick: &CommitPick) -> Result<ScopeBuild> {
    let gone = |sha: &str| {
        let status = PickStatus {
            verdict: PickVerdict::Gone(sha.to_string()),
            subject: String::new(),
            count: 0,
        };
        ScopeBuild { pick_status: Some(status), ..ScopeBuild::default() }
    };
    if !git::commit_exists(repo, &pick.newest) {
        return Ok(gone(&pick.newest));
    }
    let Some(old) = git::parent_or_empty(repo, &pick.oldest) else {
        return Ok(gone(&pick.oldest));
    };
    if old != git::EMPTY_TREE && !git::commit_exists(repo, &old) {
        return Ok(gone(&old));
    }
    let subject = git::commit_subject(repo, &pick.newest).unwrap_or_default();
    let count = git::run_length_from(repo, &old, &pick.oldest, &pick.newest).unwrap_or(0);
    let at = at_ends(repo, DiffEnds { old, new: Some(pick.newest.clone()) })?;
    // The oldest is an ancestor of the newest, so one reachability check covers the run.
    let verdict = if git::is_reachable(repo, &pick.newest) {
        PickVerdict::Live
    } else {
        PickVerdict::OffBranch
    };
    Ok(ScopeBuild { pick_status: Some(PickStatus { verdict, subject, count }), ..at })
}

/// The `All files` entries; an ignored directory is walked only once expanded.
pub(crate) fn all_files_entries(
    input: &WorldInput,
    changed: &BTreeMap<String, ChangedFile>,
) -> Result<Vec<Entry>> {
    if input.repos.is_folder() {
        let names = input.repos.names();
        let listed = crate::repos::par_map(names, |name| repo_files(input, changed, name));
        let mut entries = Vec::new();
        for (name, listed) in names.iter().zip(listed) {
            entries.extend(listed.with_context(|| name.clone())?);
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        return Ok(entries);
    }
    let to_entry = |w: git::WorktreeEntry| {
        let annotation = changed.get(&w.path).cloned();
        Entry::from_worktree(w, annotation)
    };
    let mut entries: Vec<Entry> = git::all_files(&input.repo)?.into_iter().map(&to_entry).collect();
    let mut i = 0;
    while i < entries.len() {
        if entries[i].is_dir && input.toggled_dirs.contains(&entries[i].path) {
            let path = entries[i].path.clone();
            let children = git::list_ignored_dir(&input.repo, &path).into_iter().map(&to_entry);
            entries.extend(children);
        }
        i += 1;
    }
    // Sorted by path, which a path-limited rebuild merges into in one pass.
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    entries.dedup_by(|a, b| a.path == b.path);
    Ok(entries)
}

/// The folder's repository `name`'s `All files` entries, spelled from the folder; `changed` is
/// the folder's, its annotations kept as they are.
pub(crate) fn repo_files(
    input: &WorldInput,
    changed: &BTreeMap<String, ChangedFile>,
    name: &str,
) -> Result<Vec<Entry>> {
    let inside = format!("{name}/");
    let changed: BTreeMap<String, ChangedFile> = changed
        .range(inside.clone()..)
        .take_while(|(path, _)| path.starts_with(&inside))
        .map(|(path, f)| (path[inside.len()..].to_string(), f.clone()))
        .collect();
    let entries = all_files_entries(&input.of_repo(name), &changed)?;
    let spelled = |e: Entry| Entry { path: crate::repos::join(name, &e.path), ..e };
    Ok(entries.into_iter().map(spelled).collect())
}

/// What a narrow re-read must also cover so git pairs renames as a full build would: both sides
/// of every rename it touches, and the deletions and additions it could pair with. `None` past the cap.
fn rename_partners(
    last: &BTreeMap<String, ChangedFile>,
    paths: &BTreeSet<String>,
    fresh: &[ChangedFile],
) -> Option<BTreeSet<String>> {
    let found =
        |kind| fresh.iter().any(|f: &ChangedFile| f.kind == kind || f.kind == ChangeKind::Renamed);
    let (added, deleted) = (found(ChangeKind::Added), found(ChangeKind::Deleted));
    let mut partners = BTreeSet::new();
    for f in last.values() {
        let touched = f.touched_by(paths);
        let pairs = match f.kind {
            ChangeKind::Deleted => added,
            ChangeKind::Added => deleted,
            ChangeKind::Renamed => added || deleted,
            _ => false,
        };
        if (touched && f.previous_path.is_some()) || (pairs && !git::covered(paths, &f.path)) {
            partners.insert(f.path.clone());
            partners.extend(f.previous_path.clone());
        }
    }
    partners.retain(|p| !paths.contains(p));
    let all: Vec<String> = paths.iter().chain(&partners).cloned().collect();
    git::fits_command_line(&all).then_some(partners)
}

/// Re-read only `paths` against `last`, a build of the same input, widened to rename partners.
/// `None` when that reaches past the cap: only a full build is right.
fn rebuild_paths(
    input: &WorldInput,
    last: &WorldSnapshot,
    paths: &BTreeSet<String>,
) -> Option<Result<WorldSnapshot>> {
    let (fresh, ends) = match read_paths(input, last, paths)? {
        Ok(read) => read,
        Err(e) => return Some(Err(e)),
    };
    let partners = rename_partners(&last.changeset.files, paths, &fresh)?;
    if partners.is_empty() {
        return Some(merge_paths(input, last, paths, fresh, ends));
    }
    let wide: BTreeSet<String> = paths.union(&partners).cloned().collect();
    let (fresh, ends) = match read_paths(input, last, &wide)? {
        Ok(read) => read,
        Err(e) => return Some(Err(e)),
    };
    Some(merge_paths(input, last, &wide, fresh, ends))
}

/// The changed files under `paths` as `last`'s scope reads them, and the ends they diff between.
/// `None` when last-turn's re-read files pass the cap.
fn read_paths(
    input: &WorldInput,
    last: &WorldSnapshot,
    paths: &BTreeSet<String>,
) -> Option<Result<(Vec<ChangedFile>, Option<DiffEnds>)>> {
    let ends = last.changeset.ends.clone();
    let batch: Vec<String> = paths.iter().cloned().collect();
    let read = match (&ends, input.scope) {
        // The pick reads committed trees: a worktree batch changes nothing in it.
        (None, _) | (_, Scope::Commits) => Ok((Vec::new(), ends)),
        (Some(DiffEnds { old, new: None }), _) => {
            git::changed_from_in(&input.repo, old, &batch).map(|files| (files, ends.clone()))
        }
        // The new end is last's tree with `paths` re-read, which a full snapshot would equal.
        (Some(DiffEnds { old, new: Some(new) }), _) => {
            match git::snapshot_worktree_in(&input.repo, new, &batch) {
                Ok(None) => return None,
                Ok(Some(tree)) => git::changed_between_in(&input.repo, old, &tree, &batch)
                    .map(|files| (files, Some(DiffEnds { old: old.clone(), new: Some(tree) }))),
                Err(e) => Err(e),
            }
        }
    };
    Some(read)
}

/// Merge a path-limited re-read into `last`: what `paths` covers comes from `fresh`.
fn merge_paths(
    input: &WorldInput,
    last: &WorldSnapshot,
    paths: &BTreeSet<String>,
    fresh: Vec<ChangedFile>,
    ends: Option<DiffEnds>,
) -> Result<WorldSnapshot> {
    let files: BTreeMap<String, ChangedFile> = if input.scope == Scope::Commits {
        last.changeset.files.clone()
    } else {
        let kept = last.changeset.files.values().filter(|f| !f.touched_by(paths));
        kept.cloned().chain(fresh).map(|f| (f.path.clone(), f)).collect()
    };
    let entries = match input.tab {
        Tab::AllFiles => {
            let batch: Vec<String> = paths.iter().cloned().collect();
            let annotate = |path: &str| files.get(path).cloned();
            let mut fresh: Vec<Entry> = git::all_files_in(&input.repo, &batch)?
                .into_iter()
                .map(|w| {
                    let annotation = annotate(&w.path);
                    Entry::from_worktree(w, annotation)
                })
                .collect();
            fresh.sort_by(|a, b| a.path.cmp(&b.path));
            // Both sides are sorted: one pass keeps the order, the fresh entry winning a tie.
            let kept = last.entries.iter().filter(|e| !git::covered(paths, &e.path));
            let mut entries = Vec::with_capacity(last.entries.len() + fresh.len());
            let mut fresh = fresh.into_iter().peekable();
            for e in kept {
                while fresh.peek().is_some_and(|f| f.path <= e.path) {
                    entries.extend(fresh.next());
                }
                if entries.last().is_none_or(|l: &Entry| l.path != e.path) {
                    entries.push(Entry { annotation: annotate(&e.path), ..e.clone() });
                }
            }
            entries.extend(fresh);
            entries
        }
        _ => files.values().map(Entry::from_changed).collect(),
    };
    Ok(WorldSnapshot {
        changeset: Changeset { files, ends, ..Changeset::default() },
        entries,
        branch_base: last.branch_base.clone(),
        pick_status: last.pick_status.clone(),
        head: last.head.clone(),
        touched: Some(paths.clone()),
        repo_bases: BTreeMap::new(),
        failed: Vec::new(),
    })
}

/// What a refresh re-reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refresh {
    /// Everything.
    Full,
    /// Some worktree paths; with `index`, a full build too if what the index stages changed.
    Paths { paths: BTreeSet<String>, index: bool },
    /// A folder's repositories, each by name with what it re-reads; one not named re-reads nothing.
    Repos(BTreeMap<String, Refresh>),
}

impl Default for Refresh {
    fn default() -> Self {
        Self::Paths { paths: BTreeSet::new(), index: false }
    }
}

impl Refresh {
    /// Some worktree paths, past the command-line cap a full build.
    pub fn paths(paths: impl IntoIterator<Item = String>) -> Self {
        Self::Paths { paths: paths.into_iter().collect(), index: false }.settled()
    }

    /// Whether it re-reads nothing.
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Full => false,
            Self::Paths { paths, index } => paths.is_empty() && !index,
            Self::Repos(each) => each.values().all(Self::is_empty),
        }
    }

    pub fn is_full(&self) -> bool {
        matches!(self, Self::Full)
    }

    /// Whether it may re-read `path`, spelled from the folder in a folder's refresh.
    pub fn covers(&self, path: &str, repos: &Repos) -> bool {
        match self {
            Self::Full => true,
            // A staging change may widen into a full build.
            Self::Paths { paths, index } => *index || git::covered(paths, path),
            Self::Repos(each) => repos.locate(path).is_some_and(|(name, inner)| {
                each.get(name).is_some_and(|r| r.covers(inner, &Repos::default()))
            }),
        }
    }

    /// The paths it re-reads; `None` for a full build or a folder's.
    pub fn named_paths(&self) -> Option<&BTreeSet<String>> {
        match self {
            Self::Paths { paths, .. } => Some(paths),
            Self::Full | Self::Repos(_) => None,
        }
    }

    /// What one watcher batch asks for: a rescan, or any git change but an index rewrite, moves
    /// what every path diffs against, so only a full build is right.
    pub fn from_batch(batch: &crate::watch::Batch) -> Self {
        use crate::watch::GitChange;
        if batch.rescan || batch.git.iter().any(|g| *g != GitChange::Index) {
            return Self::Full;
        }
        let index = batch.git.contains(&GitChange::Index);
        Self::Paths { paths: batch.worktree.clone(), index }.settled()
    }

    /// What one watcher batch asks of a folder: each repository what [`Self::from_batch`] would
    /// ask of it alone. A path beside the repositories asks nothing.
    pub fn from_folder_batch(batch: &crate::watch::Batch, repos: &Repos) -> Self {
        use crate::watch::GitChange;
        if batch.rescan {
            return Self::Full;
        }
        let mut each: BTreeMap<String, Refresh> = BTreeMap::new();
        for (name, git) in &batch.repo_git {
            let refresh = if git.iter().any(|g| *g != GitChange::Index) {
                Self::Full
            } else {
                Self::Paths { paths: BTreeSet::new(), index: git.contains(&GitChange::Index) }
            };
            each.entry(name.clone()).or_default().absorb(refresh);
        }
        for path in &batch.worktree {
            if let Some((name, inner)) = repos.locate(path) {
                let refresh = Self::Paths { paths: [inner.to_string()].into(), index: false };
                each.entry(name.to_string()).or_default().absorb(refresh);
            }
        }
        Self::Repos(each).settled()
    }

    /// Fold a later request in: anything full stays full, and a folder's merge by repository.
    pub fn absorb(&mut self, other: Refresh) {
        let merged = match (std::mem::take(self), other) {
            (Self::Paths { mut paths, index }, Self::Paths { paths: more, index: also }) => {
                paths.extend(more);
                Self::Paths { paths, index: index || also }
            }
            (Self::Repos(mut each), Self::Repos(more)) => {
                for (name, refresh) in more {
                    each.entry(name).or_default().absorb(refresh);
                }
                Self::Repos(each)
            }
            // The default asks nothing, so it gives way to a folder's request either side.
            (empty, other @ Self::Repos(_)) | (other @ Self::Repos(_), empty)
                if empty.is_empty() =>
            {
                other
            }
            _ => Self::Full,
        };
        *self = merged.settled();
    }

    /// Paths past the command-line cap make it full, in a folder each repository's on its own.
    fn settled(self) -> Self {
        match self {
            Self::Paths { paths, .. } if !git::fits_command_line(&paths) => Self::Full,
            Self::Repos(each) => {
                Self::Repos(each.into_iter().map(|(name, r)| (name, r.settled())).collect())
            }
            other => other,
        }
    }

    /// What it asks of the folder's repository `name`.
    fn of_repo(&self, name: &str) -> Self {
        match self {
            Self::Full => Self::Full,
            Self::Repos(each) => each.get(name).cloned().unwrap_or_default(),
            // A lone repository's request names no repository of a folder.
            Self::Paths { .. } => Self::default(),
        }
    }
}

/// One queued refresh's attributes, accumulated on `App` until the loop dispatches it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorldRequest {
    /// Re-reveal the cursor when the result lands — user-initiated switches only.
    pub reveal: bool,
    pub refresh: Refresh,
    /// Only the pacer asked for it: it spends the budget. The reviewer's work never does.
    pub background: bool,
}

/// One refresh request; the completion echoes its tag.
#[derive(Debug)]
pub struct WorldJob {
    pub generation: u64,
    pub input: WorldInput,
    /// Whether the result re-reveals the cursor: a user's switch does, a background one never.
    pub reveal: bool,
    pub refresh: Refresh,
    /// Files to read back once built, where the refresh may have changed them; `true` wants the text.
    pub probe: BTreeMap<String, bool>,
}

/// A finished job; no snapshot on the `PR` tab.
#[derive(Debug)]
pub struct WorldCompletion {
    pub generation: u64,
    pub input: WorldInput,
    pub reveal: bool,
    pub snapshot: Option<Result<WorldSnapshot>>,
    /// What the job re-read, so a failure can retry exactly that.
    pub refresh: Refresh,
    /// How long the build took: the pacing budget's measure, which bounds its git's CPU.
    pub took: std::time::Duration,
    /// The probed files the refresh may have changed, as they read after it.
    pub probed: BTreeMap<String, Probed>,
}

/// One file as a build read it back: what its bytes hash to, and its text when asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Probed {
    /// `None` while the file is missing or unreadable.
    pub fingerprint: Option<u64>,
    pub text: Option<Arc<str>>,
}

/// Read back each `wanted` path `refresh` may have changed, spelled from `root`.
pub fn probe(
    root: &Path,
    repos: &Repos,
    wanted: &BTreeMap<String, bool>,
    refresh: &Refresh,
) -> BTreeMap<String, Probed> {
    wanted
        .iter()
        .filter(|(path, _)| refresh.covers(path, repos))
        .map(|(path, &text)| (path.clone(), read_back(&root.join(path), text)))
        .collect()
}

/// One file's [`Probed`]; past the size budget, its size and time stand in for its bytes.
#[must_use]
pub fn read_back(path: &Path, text: bool) -> Probed {
    use std::hash::{Hash, Hasher};
    let hash = |value: &dyn Fn(&mut std::collections::hash_map::DefaultHasher)| {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        value(&mut hasher);
        hasher.finish()
    };
    let missing = Probed { fingerprint: None, text: None };
    let Ok(meta) = std::fs::metadata(path) else { return missing };
    if crate::diff::over_byte_budget(usize::try_from(meta.len()).unwrap_or(usize::MAX)) {
        let stamp = (meta.len(), meta.modified().ok());
        return Probed { fingerprint: Some(hash(&|h| stamp.hash(h))), text: None };
    }
    match std::fs::read(path) {
        Ok(bytes) => Probed {
            fingerprint: Some(hash(&|h| bytes.hash(h))),
            text: text.then(|| String::from_utf8_lossy(&bytes).into()),
        },
        Err(_) => missing,
    }
}

/// Run the world worker for `repo`; queued requests coalesce into the newest, keeping a reveal.
pub fn spawn(
    repo: PathBuf,
    rx: Receiver<WorldJob>,
    tx: crate::wake::Sender<WorldCompletion>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("world".into())
        .spawn(move || {
            git::sweep_dead_copies(&repo);
            let mut worker = Worker::default();
            while let Ok(mut job) = rx.recv() {
                while let Ok(next) = rx.try_recv() {
                    let mut refresh = job.refresh;
                    refresh.absorb(next.refresh);
                    job = WorldJob { reveal: job.reveal || next.reveal, refresh, ..next };
                }
                let started = std::time::Instant::now();
                let snapshot = if job.input.tab.is_file_tab() {
                    worker.refresh(&job.input, job.refresh.clone())
                } else {
                    None
                };
                let probed = match &snapshot {
                    Some(Ok(_)) => {
                        probe(&job.input.repo, &job.input.repos, &job.probe, &job.refresh)
                    }
                    _ => BTreeMap::new(),
                };
                let done = WorldCompletion {
                    generation: job.generation,
                    input: job.input,
                    reveal: job.reveal,
                    snapshot,
                    refresh: job.refresh,
                    took: started.elapsed(),
                    probed,
                };
                if tx.send(done).is_err() {
                    break;
                }
            }
        })
        .expect("spawn world worker")
}

/// What the worker keeps between jobs: its last build, and what the index staged when it ran.
#[derive(Default)]
struct Worker {
    last: Option<(WorldInput, WorldSnapshot)>,
    staged: Option<u64>,
    /// A folder's repositories by name: each one's own worker and its last good build.
    repos: BTreeMap<String, (Worker, Option<WorldSnapshot>)>,
}

impl Worker {
    /// Run one refresh: only its paths when the last build is of the same input, else in full.
    /// `None` when nothing needs re-reading.
    fn refresh(
        &mut self,
        input: &WorldInput,
        mut refresh: Refresh,
    ) -> Option<Result<WorldSnapshot>> {
        if input.repos.is_folder() {
            return self.refresh_folder(input, &refresh);
        }
        // A pick reads committed trees: on `Changes`, a worktree or index change moves nothing in it.
        let pick_only = input.scope == Scope::Commits && input.tab == Tab::Changes;
        if refresh.is_empty() || (pick_only && !refresh.is_full()) {
            return None;
        }
        // Read before any build, so a stage landing during it reads as new next time.
        let mut fingerprint = None;
        if let Refresh::Paths { index: true, .. } = refresh {
            let now = git::staged_fingerprint(&input.repo).ok();
            if now.is_none() || now != self.staged {
                refresh = Refresh::Full;
                fingerprint = Some(now);
            }
        }
        // An expanded ignored directory lists from disk, so a batch inside one rebuilds it whole.
        let in_expanded = |p: &String| {
            input.toggled_dirs.iter().any(|d| git::under_or_eq(p, d) || git::under_or_eq(d, p))
        };
        let last = self.last.as_ref().filter(|(of, _)| of == input).map(|(_, built)| built);
        if let Some(last) = last
            && let Refresh::Paths { paths, .. } = &refresh
            && !paths.iter().any(in_expanded)
        {
            if paths.is_empty() {
                return None;
            }
            match rebuild_paths(input, last, paths) {
                Some(Ok(built)) => {
                    self.last = Some((input.clone(), built.clone()));
                    return Some(Ok(built));
                }
                // A failed narrow read is retried at once as a full build.
                Some(Err(e)) => logln!("path-limited refresh failed, rebuilding in full: {e:#}"),
                None => {}
            }
        }
        let fingerprint = fingerprint.unwrap_or_else(|| git::staged_fingerprint(&input.repo).ok());
        let built = build(input);
        match &built {
            Ok(built) => {
                self.staged = fingerprint;
                self.last = Some((input.clone(), built.clone()));
            }
            // A failed build saw nothing: the next one reads everything again.
            Err(_) => self.last = None,
        }
        Some(built)
    }

    /// Refresh a folder: each repository re-reads only what `refresh` asks of it, on a few threads.
    fn refresh_folder(
        &mut self,
        input: &WorldInput,
        refresh: &Refresh,
    ) -> Option<Result<WorldSnapshot>> {
        // A repository that left the folder takes its files with it.
        let left = self.repos.keys().any(|name| !input.repos.contains(name));
        self.repos.retain(|name, _| input.repos.contains(name));
        let slots: Vec<std::sync::Mutex<(String, Worker, Option<WorldSnapshot>)>> = input
            .repos
            .names()
            .iter()
            .map(|name| {
                let (worker, good) = self.repos.remove(name).unwrap_or_default();
                std::sync::Mutex::new((name.clone(), worker, good))
            })
            .collect();
        let read = crate::repos::par_map(&slots, |slot| {
            let mut slot = slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let (name, worker, good) = &mut *slot;
            let input = input.of_repo(name);
            // One never built for this input, or whose last build failed, reads everything.
            let fresh = good.is_some() && worker.last.as_ref().is_some_and(|(of, _)| *of == input);
            let refresh = if fresh { refresh.of_repo(name) } else { Refresh::Full };
            if worker.repos.is_empty() && worker.last.is_none() && good.is_none() {
                git::sweep_dead_copies(&input.repo);
            }
            match worker.refresh(&input, refresh) {
                Some(Ok(built)) => {
                    let touched = built.touched.clone();
                    *good = Some(built);
                    Some(Ok(touched))
                }
                Some(Err(e)) => Some(Err(format!("{e:#}"))),
                None => None,
            }
        });
        let mut parts = BTreeMap::new();
        let mut touched = Some(BTreeSet::new());
        let mut failed = Vec::new();
        let mut any = left;
        for (slot, read) in slots.into_iter().zip(read) {
            let (name, worker, good) =
                slot.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner);
            match read {
                Some(Ok(Some(paths))) => {
                    if let Some(all) = touched.as_mut() {
                        all.extend(paths.iter().map(|p| crate::repos::join(&name, p)));
                    }
                    any = true;
                }
                // A full build touched the whole repository, which its name covers.
                Some(Ok(None)) => {
                    if let Some(all) = touched.as_mut() {
                        all.insert(name.clone());
                    }
                    any = true;
                }
                Some(Err(why)) => {
                    failed.push((name.clone(), why));
                    any = true;
                }
                None => {}
            }
            if let Some(built) = &good {
                parts.insert(name.clone(), built.clone());
            }
            self.repos.insert(name, (worker, good));
        }
        if !any {
            return None;
        }
        // A full refresh re-read everything, which `None` says.
        let touched = touched.filter(|_| !refresh.is_full());
        Some(Ok(compose(&parts, touched, failed)))
    }
}

#[cfg(test)]
mod tests {
    use super::Refresh;
    use crate::watch::{Batch, GitChange};

    fn batch(git: &[GitChange], worktree: &[&str], rescan: bool) -> Batch {
        Batch {
            worktree: worktree.iter().map(|p| (*p).to_string()).collect(),
            git: git.iter().copied().collect(),
            rescan,
            ..Batch::default()
        }
    }

    #[test]
    fn a_batch_asks_for_exactly_what_it_moved() {
        let paths = |list: &[&str]| list.iter().map(|p| (*p).to_string()).collect();
        let rows = [
            ("a rescan", batch(&[], &["a"], true), Refresh::Full),
            ("HEAD", batch(&[GitChange::Head], &["a"], false), Refresh::Full),
            ("a ref", batch(&[GitChange::Refs], &[], false), Refresh::Full),
            ("git config", batch(&[GitChange::Config], &[], false), Refresh::Full),
            ("an ignore rule", batch(&[GitChange::IgnoreRules], &[], false), Refresh::Full),
            ("an attribute rule", batch(&[GitChange::Attributes], &[], false), Refresh::Full),
            ("another pane's pick", batch(&[GitChange::ReviewrRefs], &[], false), Refresh::Full),
            (
                "an index rewrite with paths",
                batch(&[GitChange::Index], &["a"], false),
                Refresh::Paths { paths: paths(&["a"]), index: true },
            ),
            (
                "worktree paths alone",
                batch(&[], &["a", "b/c"], false),
                Refresh::Paths { paths: paths(&["a", "b/c"]), index: false },
            ),
            ("the config alone", Batch { config: true, ..Batch::default() }, Refresh::default()),
        ];
        for (name, batch, expected) in rows {
            assert_eq!(Refresh::from_batch(&batch), expected, "{name}");
        }
    }

    fn each(rows: &[(&str, Refresh)]) -> Refresh {
        Refresh::Repos(rows.iter().map(|(n, r)| ((*n).to_string(), r.clone())).collect())
    }

    fn some(paths: &[&str], index: bool) -> Refresh {
        Refresh::Paths { paths: paths.iter().map(|p| (*p).to_string()).collect(), index }
    }

    #[test]
    fn a_folder_batch_asks_each_repository_for_what_moved_in_it() {
        let repos = crate::repos::Repos::of(&["api", "web"]);
        let with_git = |git: &[(&str, &[GitChange])], worktree: &[&str]| Batch {
            repo_git: git
                .iter()
                .map(|(n, g)| ((*n).to_string(), g.iter().copied().collect()))
                .collect(),
            ..batch(&[], worktree, false)
        };
        let rows = [
            (
                "paths in two repositories, one beside them",
                with_git(&[], &["api/a.rs", "web/b/c.rs", "notes.txt"]),
                each(&[("api", some(&["a.rs"], false)), ("web", some(&["b/c.rs"], false))]),
            ),
            (
                "a stage in one",
                with_git(&[("web", &[GitChange::Index])], &["web/b.rs"]),
                each(&[("web", some(&["b.rs"], true))]),
            ),
            (
                "a commit in one",
                with_git(&[("web", &[GitChange::Head, GitChange::Index])], &["api/a.rs"]),
                each(&[("api", some(&["a.rs"], false)), ("web", Refresh::Full)]),
            ),
            ("lost events", Batch { rescan: true, ..with_git(&[], &["api/a.rs"]) }, Refresh::Full),
        ];
        for (name, batch, expected) in rows {
            assert_eq!(Refresh::from_folder_batch(&batch, &repos), expected, "{name}");
        }
    }

    #[test]
    fn a_folders_requests_merge_by_repository_and_anything_full_stays_full() {
        let mut merged = each(&[("api", some(&["a"], false))]);
        merged.absorb(each(&[("api", some(&["b"], true)), ("web", Refresh::Full)]));
        assert_eq!(merged, each(&[("api", some(&["a", "b"], true)), ("web", Refresh::Full)]));
        // The default asks nothing, so it gives way either side.
        let mut from_default = Refresh::default();
        from_default.absorb(each(&[("api", some(&["a"], false))]));
        assert_eq!(from_default, each(&[("api", some(&["a"], false))]));
        let mut into_default = each(&[("api", some(&["a"], false))]);
        into_default.absorb(Refresh::default());
        assert_eq!(into_default, each(&[("api", some(&["a"], false))]));
        let mut full = Refresh::Full;
        full.absorb(each(&[("api", some(&["a"], false))]));
        assert_eq!(full, Refresh::Full);
        assert!(each(&[]).is_empty() && each(&[("api", Refresh::default())]).is_empty());
        assert!(!each(&[("api", some(&[], true))]).is_empty(), "a stage re-reads");
    }
}
