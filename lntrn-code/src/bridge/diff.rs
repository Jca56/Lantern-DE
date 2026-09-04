//! A line diff (Myers, on the lines left after the common head and tail
//! are trimmed), as rows for a unified view.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Same,
    Removed,
    Added,
}

/// One row of a unified diff: which lines of the old and new text it
/// shows (indices), and what happened to them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub kind: Kind,
    pub old: Option<usize>,
    pub new: Option<usize>,
}

/// Past this many edits the diff is shown as everything removed and
/// everything added, instead of grinding.
const MAX_EDITS: usize = 20_000;

pub fn diff_lines(old: &[&str], new: &[&str]) -> Vec<Row> {
    let mut head = 0;
    while head < old.len() && head < new.len() && old[head] == new[head] {
        head += 1;
    }
    let mut tail = 0;
    while tail < old.len() - head && tail < new.len() - head && old[old.len() - 1 - tail] == new[new.len() - 1 - tail] {
        tail += 1;
    }
    let mut rows: Vec<Row> = (0..head).map(|i| Row { kind: Kind::Same, old: Some(i), new: Some(i) }).collect();
    let (a, b) = (&old[head..old.len() - tail], &new[head..new.len() - tail]);
    match myers(a, b) {
        Some(edits) => {
            let (mut i, mut j) = (0, 0);
            for e in edits {
                match e {
                    Kind::Same => {
                        rows.push(Row { kind: Kind::Same, old: Some(head + i), new: Some(head + j) });
                        i += 1;
                        j += 1;
                    }
                    Kind::Removed => {
                        rows.push(Row { kind: Kind::Removed, old: Some(head + i), new: None });
                        i += 1;
                    }
                    Kind::Added => {
                        rows.push(Row { kind: Kind::Added, old: None, new: Some(head + j) });
                        j += 1;
                    }
                }
            }
        }
        None => {
            rows.extend((0..a.len()).map(|i| Row { kind: Kind::Removed, old: Some(head + i), new: None }));
            rows.extend((0..b.len()).map(|j| Row { kind: Kind::Added, old: None, new: Some(head + j) }));
        }
    }
    let (on, nn) = (old.len(), new.len());
    rows.extend((0..tail).map(|k| Row { kind: Kind::Same, old: Some(on - tail + k), new: Some(nn - tail + k) }));
    rows
}

/// The edit script between `a` and `b`, or `None` past the edit cap.
fn myers(a: &[&str], b: &[&str]) -> Option<Vec<Kind>> {
    let (n, m) = (a.len() as isize, b.len() as isize);
    if n == 0 && m == 0 {
        return Some(Vec::new());
    }
    let max = (n + m).min(MAX_EDITS as isize);
    let offset = max;
    let width = (2 * max + 1) as usize;
    let mut v = vec![0isize; width];
    let mut trace: Vec<Vec<isize>> = Vec::new();
    let mut found = None;
    'outer: for d in 0..=max {
        trace.push(v.clone());
        let mut k = -d;
        while k <= d {
            let idx = (k + offset) as usize;
            let mut x = if k == -d || (k != d && v[idx - 1] < v[idx + 1]) { v[idx + 1] } else { v[idx - 1] + 1 };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[idx] = x;
            if x >= n && y >= m {
                found = Some(d);
                break 'outer;
            }
            k += 2;
        }
    }
    let d_final = found?;
    // Walk the trace back to build the script.
    let mut script = Vec::new();
    let (mut x, mut y) = (n, m);
    for d in (1..=d_final).rev() {
        let v = &trace[d as usize];
        let k = x - y;
        let idx = (k + offset) as usize;
        let prev_k = if k == -d || (k != d && v[idx - 1] < v[idx + 1]) { k + 1 } else { k - 1 };
        let prev_x = v[(prev_k + offset) as usize];
        let prev_y = prev_x - prev_k;
        while x > prev_x && y > prev_y {
            script.push(Kind::Same);
            x -= 1;
            y -= 1;
        }
        if x == prev_x {
            script.push(Kind::Added);
        } else {
            script.push(Kind::Removed);
        }
        x = prev_x;
        y = prev_y;
    }
    while x > 0 && y > 0 {
        script.push(Kind::Same);
        x -= 1;
        y -= 1;
    }
    script.reverse();
    Some(script)
}

/// How many lines were added and removed.
pub fn counts(rows: &[Row]) -> (usize, usize) {
    (rows.iter().filter(|r| r.kind == Kind::Added).count(), rows.iter().filter(|r| r.kind == Kind::Removed).count())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(old: &[&str], new: &[&str]) -> String {
        diff_lines(old, new)
            .iter()
            .map(|r| match r.kind {
                Kind::Same => format!(" {}", old[r.old.unwrap()]),
                Kind::Removed => format!("-{}", old[r.old.unwrap()]),
                Kind::Added => format!("+{}", new[r.new.unwrap()]),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn diffs() {
        assert_eq!(render(&["a", "b", "c"], &["a", "x", "c"]), " a\n-b\n+x\n c");
        assert_eq!(render(&["a", "b"], &["a", "b", "c"]), " a\n b\n+c");
        assert_eq!(render(&["a", "b", "c"], &["b"]), "-a\n b\n-c");
        assert_eq!(render(&[], &["n"]), "+n");
        assert_eq!(render(&["x"], &[]), "-x");
        assert_eq!(render(&["s"], &["s"]), " s");
        let rows = diff_lines(&["a", "b", "c", "d"], &["a", "c", "b", "d"]);
        assert_eq!(counts(&rows), (1, 1));
        // The rows reproduce both sides in order.
        let olds: Vec<usize> = rows.iter().filter_map(|r| r.old).collect();
        let news: Vec<usize> = rows.iter().filter_map(|r| r.new).collect();
        assert_eq!(olds, vec![0, 1, 2, 3]);
        assert_eq!(news, vec![0, 1, 2, 3]);
    }
}
