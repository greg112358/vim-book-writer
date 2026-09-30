//! Line differences against a base text, for the gutter marks that show what changed.

/// How a line differs from the base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Added,
    Modified,
    /// Lines were taken out just above this one.
    Removed,
}

/// The most edits worth looking for before giving up and calling the whole stretch modified.
const MAX_EDITS: usize = 400;

#[derive(Clone, Copy, PartialEq)]
enum Edit {
    Keep,
    Delete,
    Insert,
}

/// The shortest edit script from `a` to `b`, by Myers' algorithm, or `None` if it needs more
/// than `MAX_EDITS` changes.
fn myers(a: &[&str], b: &[&str]) -> Option<Vec<Edit>> {
    let (n, m) = (a.len() as isize, b.len() as isize);
    let cap = (n + m).min(MAX_EDITS as isize);
    let offset = cap + 1;
    let mut v = vec![0isize; (2 * cap + 3) as usize];
    let mut trace: Vec<Vec<isize>> = Vec::new();
    for d in 0..=cap {
        trace.push(v.clone());
        for k in (-d..=d).step_by(2) {
            let at = |k: isize| (k + offset) as usize;
            let mut x = if k == -d || (k != d && v[at(k - 1)] < v[at(k + 1)]) {
                v[at(k + 1)]
            } else {
                v[at(k - 1)] + 1
            };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[at(k)] = x;
            if x >= n && y >= m {
                // Walk back through the rounds to read off the edits.
                let mut edits = Vec::new();
                let (mut x, mut y) = (n, m);
                for d in (0..=d).rev() {
                    let v = &trace[d as usize];
                    let k = x - y;
                    let prev_k = if k == -d || (k != d && v[at(k - 1)] < v[at(k + 1)]) {
                        k + 1
                    } else {
                        k - 1
                    };
                    let (prev_x, prev_y) = (v[at(prev_k)], v[at(prev_k)] - prev_k);
                    while x > prev_x && y > prev_y {
                        edits.push(Edit::Keep);
                        x -= 1;
                        y -= 1;
                    }
                    if d > 0 {
                        edits.push(if x == prev_x {
                            Edit::Insert
                        } else {
                            Edit::Delete
                        });
                    }
                    (x, y) = (prev_x, prev_y);
                }
                edits.reverse();
                return Some(edits);
            }
        }
    }
    None
}

/// The lines of `text` (counting from 0) that differ from `base`, in order: lines that are new,
/// lines that replace others, and lines with some taken out just above them.
pub fn marks(base: &str, text: &str) -> Vec<(usize, Change)> {
    let (a, b): (Vec<&str>, Vec<&str>) = (base.lines().collect(), text.lines().collect());
    // What both ends share is not worth searching.
    let head = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let tail = a[head..]
        .iter()
        .rev()
        .zip(b[head..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (a, b) = (&a[head..a.len() - tail], &b[head..b.len() - tail]);
    let edits = myers(a, b).unwrap_or_else(|| {
        // Too different to say which lines match: the old ones out and the new ones in.
        let mut all = vec![Edit::Delete; a.len()];
        all.extend(vec![Edit::Insert; b.len()]);
        all
    });
    let mut out = Vec::new();
    let (mut line, mut i) = (head, 0);
    while i < edits.len() {
        if edits[i] == Edit::Keep {
            line += 1;
            i += 1;
            continue;
        }
        let end = i + edits[i..].iter().take_while(|&&e| e != Edit::Keep).count();
        let deleted = edits[i..end].iter().filter(|&&e| e == Edit::Delete).count();
        let inserted = end - i - deleted;
        if inserted == 0 {
            out.push((line, Change::Removed));
        }
        for j in 0..inserted {
            out.push((
                line + j,
                if j < deleted {
                    Change::Modified
                } else {
                    Change::Added
                },
            ));
        }
        line += inserted;
        i = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use Change::*;

    #[test]
    fn lines_are_added_modified_or_have_some_removed_above() {
        let base = "a\nb\nc\nd\ne\n";
        for (text, want) in [
            (base, vec![]),
            ("a\nb\nX\nd\ne\n", vec![(2, Modified)]),
            ("a\nb\nc\nnew\nd\ne\n", vec![(3, Added)]),
            ("a\nb\nd\ne\n", vec![(2, Removed)]),
            (
                "a\nb\nX\nY\nZ\nd\ne\n",
                vec![(2, Modified), (3, Added), (4, Added)],
            ),
            ("new\na\nb\nc\nd\ne\n", vec![(0, Added)]),
            ("a\nb\nc\nd\ne\nend\n", vec![(5, Added)]),
            ("b\nc\nd\ne\n", vec![(0, Removed)]),
            ("a\nX\nc\nd\nY\n", vec![(1, Modified), (4, Modified)]),
        ] {
            assert_eq!(marks(base, text), want, "{text:?}");
        }
        // Everything is new against nothing, and nothing is left against nothing.
        assert_eq!(marks("", "a\nb\n"), [(0, Added), (1, Added)]);
        assert!(marks("", "").is_empty());
    }

    #[test]
    fn a_rewrite_too_big_to_match_is_all_modified() {
        let base: String = (0..500).map(|i| format!("old {i}\n")).collect();
        let text: String = (0..500).map(|i| format!("new {i}\n")).collect();
        let found = marks(&base, &text);
        assert_eq!(found.len(), 500);
        assert!(found.iter().all(|&(_, c)| c == Modified));
    }
}
