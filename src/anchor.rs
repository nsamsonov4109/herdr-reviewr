//! Where a comment's lines went when their file changed: carried, found again, or changed under it.

use std::hash::{Hash, Hasher};

/// `text`'s lines without their endings, a CR before the `\n` dropped, as anchors compare them.
#[must_use]
pub fn lines(text: &str) -> Vec<&str> {
    crate::text::lines(text).into_iter().map(|line| crate::text::line_body(line).0).collect()
}

/// What `lines` read as: equal versions for equal lines, whatever their endings.
#[must_use]
pub fn version(lines: &[&str]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    lines.hash(&mut hasher);
    hasher.finish()
}

/// Old → new line numbers across one edit; a changed line maps to its nearest survivor.
#[derive(Debug)]
pub struct LineMap {
    ops: Vec<similar::DiffOp>,
    new_len: usize,
}

impl LineMap {
    /// The map from `old` to `new`, each split as git counts lines, endings kept.
    #[must_use]
    pub fn new(old: &str, new: &str) -> Self {
        Self::of_lines(&crate::text::lines(old), &crate::text::lines(new))
    }

    /// The map between two texts already split into lines.
    #[must_use]
    pub fn of_lines(old: &[&str], new: &[&str]) -> Self {
        let ops = similar::TextDiff::from_slices(old, new).ops().to_vec();
        Self { ops, new_len: new.len() }
    }

    /// Map 1-based old line `line` to its new line, and whether it is there, the same or
    /// rewritten in place, rather than deleted.
    #[must_use]
    pub fn place(&self, line: u32) -> (u32, bool) {
        let i = line.saturating_sub(1) as usize;
        let op = self.ops.iter().find(|op| op.as_tag_tuple().1.contains(&i));
        let kept = op.is_some_and(|op| op.as_tag_tuple().0 != similar::DiffTag::Delete);
        (self.line(line), kept)
    }

    /// Map 1-based old line `line` to its 1-based new line.
    #[must_use]
    pub fn line(&self, line: u32) -> u32 {
        let i = line.saturating_sub(1) as usize;
        let mapped = self.ops.iter().find_map(|op| {
            let (tag, old, new) = op.as_tag_tuple();
            old.contains(&i).then(|| match tag {
                similar::DiffTag::Equal => new.start + (i - old.start),
                _ => new.start + (i - old.start).min(new.len().saturating_sub(1)),
            })
        });
        let mapped = mapped.unwrap_or(self.new_len).min(self.new_len.saturating_sub(1));
        mapped as u32 + 1
    }
}

/// Where a comment's lines sit now, and whether they still read as they did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placed {
    pub start: u32,
    pub end: u32,
    /// The lines under the comment no longer read as the ones it was made on.
    pub changed: bool,
}

/// Fewer visible characters than this, and a lone match elsewhere proves nothing (`}`, `else {`).
const DISTINCT: usize = 8;

/// Carry `held` (lines `start..=end` of `before`) into `now`: through the edit while they read the
/// same, else to their one match, else where the edit put them, changed.
#[must_use]
pub fn follow(
    held: &[String],
    start: u32,
    end: u32,
    before: Option<&[&str]>,
    now: &[&str],
) -> Placed {
    let (mapped_start, mapped_end) = match before {
        Some(before) => {
            let map = LineMap::of_lines(before, now);
            (map.line(start), map.line(end))
        }
        None => (start, end),
    };
    let last = (now.len() as u32).max(1);
    let start = mapped_start.clamp(1, last);
    let end = mapped_end.clamp(start, last);
    if held.is_empty() || reads_at(now, start, held) {
        let end = if held.is_empty() { end } else { start + held.len() as u32 - 1 };
        return Placed { start, end, changed: false };
    }
    match moved_to(held, before, now) {
        Some(at) => Placed { start: at, end: at + held.len() as u32 - 1, changed: false },
        None => Placed { start, end, changed: true },
    }
}

/// Where another view shows `held`, lines `start..=end` of `before` if known, in `now`: where the
/// edit put them while they read the same, else their one match, else where it rewrote them.
#[must_use]
pub fn locate(
    held: &[String],
    start: u32,
    end: u32,
    before: Option<&[&str]>,
    now: &[&str],
) -> Option<Placed> {
    let last = held.len().checked_sub(1)? as u32;
    let map = before.map(|before| LineMap::of_lines(before, now));
    // Without the text they were made on, a line number alone proves nothing for `}`.
    let (at, kept) = map.as_ref().map_or((start, false), |map| map.place(start));
    if reads_at(now, at, held) && (map.is_some() || distinct(held)) {
        return Some(Placed { start: at, end: at + last, changed: false });
    }
    if let Some(at) = moved_to(held, before, now) {
        return Some(Placed { start: at, end: at + last, changed: false });
    }
    let map = map.filter(|_| kept)?;
    let bound = (now.len() as u32).max(1);
    let start = at.clamp(1, bound);
    Some(Placed { start, end: map.line(end).clamp(start, bound), changed: true })
}

/// Whether `held` shows enough to prove its lines anywhere but where they were made.
fn distinct(held: &[String]) -> bool {
    let visible: usize =
        held.iter().map(|l| l.chars().filter(|c| !c.is_whitespace()).count()).sum();
    visible >= DISTINCT
}

/// Where `held` moved in `now`: its one match there, when it was one of a kind in `before` too, so
/// a rewritten line never lands on the twin it left behind.
fn moved_to(held: &[String], before: Option<&[&str]>, now: &[&str]) -> Option<u32> {
    let unique_before = before.is_none_or(|before| lone_match(held, before).is_some());
    lone_match(held, now).filter(|_| unique_before)
}

/// The one line `held` reads from in `now`, when it is distinct enough to prove anything.
fn lone_match(held: &[String], now: &[&str]) -> Option<u32> {
    if !distinct(held) {
        return None;
    }
    let mut found = (1..=now.len() as u32).filter(|&at| reads_at(now, at, held));
    match (found.next(), found.next()) {
        (Some(at), None) => Some(at),
        _ => None,
    }
}

/// Whether `now` holds `held` from 1-based line `at` on.
fn reads_at(now: &[&str], at: u32, held: &[String]) -> bool {
    let from = at.saturating_sub(1) as usize;
    now.get(from..from + held.len()).is_some_and(|run| run.iter().zip(held).all(|(a, b)| a == b))
}

#[cfg(test)]
mod tests {
    use super::{LineMap, Placed, follow, lines, locate, version};

    fn held(text: &[&str]) -> Vec<String> {
        text.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn a_line_map_carries_each_line_through_every_kind_of_edit() {
        // (old, new, old line, its new line)
        let cases = [
            ("a\nb\nc\n", "x\na\nb\nc\n", 2, 3), // equal, shifted by an insert above
            ("a\nb\nc\n", "a\nB\nc\n", 2, 2),    // replaced: its counterpart
            ("a\nb\nc\nd\n", "a\nd\n", 3, 2),    // deleted: the line after
            ("a\nb\n", "a\nb\n", 9, 2),          // past the end: the last line
            ("a\nb\n", "", 1, 1),                // an emptied file: line 1
            ("a\rb\nc\n", "a\rb\nc\n", 9, 2),    // a bare CR breaks no line
        ];
        for (old, new, line, want) in cases {
            assert_eq!(LineMap::new(old, new).line(line), want, "{old:?} → {new:?} at {line}");
        }
    }

    #[test]
    fn lines_and_versions_ignore_line_endings() {
        assert_eq!(lines("a\r\nb\nc"), ["a", "b", "c"]);
        assert_eq!(version(&lines("a\r\nb\n")), version(&lines("a\nb\n")));
        assert_ne!(version(&lines("a\nb\n")), version(&lines("a\nB\n")));
    }

    #[test]
    fn a_comment_rides_an_edit_above_it() {
        let before = lines("fn a() {\n    let x = 1;\n}\n");
        let now = lines("// new\n// lines\nfn a() {\n    let x = 1;\n}\n");
        let placed = follow(&held(&["    let x = 1;"]), 2, 2, Some(&before), &now);
        assert_eq!(placed, Placed { start: 4, end: 4, changed: false });
    }

    #[test]
    fn rewritten_lines_keep_the_comment_in_place_and_say_so() {
        let before = lines("a\n    let total = items.len();\nc\n");
        let now = lines("a\n    let total = items.iter().count();\nc\n");
        let placed = follow(&held(&["    let total = items.len();"]), 2, 2, Some(&before), &now);
        assert_eq!(placed, Placed { start: 2, end: 2, changed: true });
    }

    #[test]
    fn moved_lines_are_found_again_when_they_appear_once() {
        let before = lines("fn one() {}\nfn two() { work(); }\nfn three() {}\n");
        let now = lines("fn one() {}\nfn three() {}\nfn two() { work(); }\n");
        let placed = follow(&held(&["fn two() { work(); }"]), 2, 2, Some(&before), &now);
        assert_eq!(placed, Placed { start: 3, end: 3, changed: false });
    }

    #[test]
    fn a_short_or_repeated_line_is_never_guessed_elsewhere() {
        // `}` changed to `};`: another `}` proves nothing.
        let before = lines("if a {\n}\nif b {\n}\n");
        let now = lines("if a {\n};\nif b {\n}\n");
        assert!(follow(&held(&["}"]), 2, 2, Some(&before), &now).changed);
        // Two copies of the line elsewhere: neither is it.
        let now = lines("x\n    call_it(2);\ny\n    call_it(1);\nz\n    call_it(1);\n");
        let placed = follow(&held(&["    call_it(1);"]), 2, 2, None, &now);
        assert_eq!(placed, Placed { start: 2, end: 2, changed: true });
    }

    #[test]
    fn deleted_lines_land_on_the_next_survivor_changed() {
        let before = lines("a\nb\nc\nd\n");
        let now = lines("a\nd\n");
        let placed = follow(&held(&["b", "c"]), 2, 3, Some(&before), &now);
        assert_eq!(placed, Placed { start: 2, end: 2, changed: true });
    }

    #[test]
    fn a_line_inserted_inside_the_range_spans_it_changed() {
        let before = lines("a\nfirst line here\nsecond line here\nd\n");
        let now = lines("a\nfirst line here\ninserted\nsecond line here\nd\n");
        let placed =
            follow(&held(&["first line here", "second line here"]), 2, 3, Some(&before), &now);
        assert_eq!(placed, Placed { start: 2, end: 4, changed: true });
    }

    #[test]
    fn without_the_old_text_a_comment_stays_while_its_lines_read_the_same() {
        let now = lines("a\nb\nc\n");
        assert_eq!(
            follow(&held(&["b"]), 2, 2, None, &now),
            Placed { start: 2, end: 2, changed: false }
        );
        let placed = follow(&held(&["something else"]), 2, 2, None, &now);
        assert_eq!(placed, Placed { start: 2, end: 2, changed: true });
        // An emptied file clamps to its first line.
        assert_eq!(
            follow(&held(&["b"]), 2, 2, None, &[]),
            Placed { start: 1, end: 1, changed: true }
        );
    }

    #[test]
    fn another_view_shows_the_lines_where_they_read_the_same_or_nowhere() {
        let at = |start, end| Some(Placed { start, end, changed: false });
        let now = lines("a\n    let total = items.len();\nc\n    let total = items.len();\n");
        let total = held(&["    let total = items.len();"]);
        assert_eq!(
            locate(&total, 2, 2, None, &now),
            at(2, 2),
            "at its own line while it reads there"
        );
        assert_eq!(locate(&total, 1, 1, None, &now), None, "two matches elsewhere: neither is it");
        let now = lines("x\ny\n    let total = items.len();\n");
        assert_eq!(locate(&total, 1, 1, None, &now), at(3, 3), "its one match");
        // A short line proves nothing by its number alone, only through the text it was made on.
        let brace = held(&["}"]);
        assert_eq!(locate(&brace, 2, 2, None, &lines("{\n}\n")), None);
        let made_on = lines("{\n}\n");
        assert_eq!(locate(&brace, 2, 2, Some(&made_on), &lines("x\n{\n}\n")), at(3, 3));
        assert_eq!(locate(&[], 1, 1, None, &now), None);
    }

    #[test]
    fn a_rewritten_line_shows_where_it_went_not_where_a_twin_now_sits() {
        // An amend rewrote the commented `return;` and moved it; another `return;` took its number.
        let made_on = lines("fn first() {\n  if a {\n    return;\n  }\n}\n");
        let now = lines(
            "fn zero() {\n  if z {\n    return;\n  }\n}\n\
             fn first() {\n  if a {\n    return;  // not ours\n  }\n}\n",
        );
        let placed = locate(&held(&["    return;"]), 3, 3, Some(&made_on), &now);
        assert_eq!(placed, Some(Placed { start: 8, end: 8, changed: true }));
        // Deleted outright, it shows nowhere.
        let made_on = lines("a\nkeep this line here\nb\n");
        let gone = locate(&held(&["keep this line here"]), 2, 2, Some(&made_on), &lines("a\nb\n"));
        assert_eq!(gone, None);
    }

    #[test]
    fn a_rewritten_or_deleted_line_never_lands_on_its_twin() {
        let made_on = "fn a() {\n    cleanup_resources(handle);\n}\nfn b() {\n    cleanup_resources(handle);\n}\n";
        let before = lines(made_on);
        let line = held(&["    cleanup_resources(handle);"]);
        // Rewritten: it stays on its own line, changed, never on fn b's copy.
        let now = lines(
            "fn a() {\n    release(handle);\n}\nfn b() {\n    cleanup_resources(handle);\n}\n",
        );
        assert_eq!(
            follow(&line, 2, 2, Some(&before), &now),
            Placed { start: 2, end: 2, changed: true }
        );
        assert_eq!(
            locate(&line, 2, 2, Some(&before), &now),
            Some(Placed { start: 2, end: 2, changed: true })
        );
        // Deleted: where the edit put it, changed.
        let now = lines("fn a() {\n}\nfn b() {\n    cleanup_resources(handle);\n}\n");
        let placed = follow(&line, 2, 2, Some(&before), &now);
        assert!(placed.changed && placed.start != 4, "{placed:?}");
        assert_ne!(locate(&line, 2, 2, Some(&before), &now).map(|p| p.start), Some(4));
    }
}
