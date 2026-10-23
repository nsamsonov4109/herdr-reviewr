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
    let visible: usize =
        held.iter().map(|l| l.chars().filter(|c| !c.is_whitespace()).count()).sum();
    if visible >= DISTINCT {
        let mut found = (1..=now.len() as u32).filter(|&at| reads_at(now, at, held));
        if let (Some(at), None) = (found.next(), found.next()) {
            return Placed { start: at, end: at + held.len() as u32 - 1, changed: false };
        }
    }
    Placed { start, end, changed: true }
}

/// Whether `now` holds `held` from 1-based line `at` on.
fn reads_at(now: &[&str], at: u32, held: &[String]) -> bool {
    let from = at.saturating_sub(1) as usize;
    now.get(from..from + held.len()).is_some_and(|run| run.iter().zip(held).all(|(a, b)| a == b))
}

#[cfg(test)]
mod tests {
    use super::{LineMap, Placed, follow, lines, version};

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
}
