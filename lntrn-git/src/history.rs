//! The history graph's layout: which lane each commit sits in and the
//! lines from it to its parents. Commits come newest first, children
//! before parents, as `git log --topo-order` gives them.

use std::collections::HashMap;

use crate::git::Commit;

/// A line from a commit down to one of its parents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edge {
    /// The lane it leaves in (the commit's own) and the one it arrives in.
    pub from: usize,
    pub to: usize,
    /// The parent's row.
    pub row: usize,
}

/// Where one commit sits.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Node {
    pub lane: usize,
    pub edges: Vec<Edge>,
}

/// The commits and where each one is drawn.
#[derive(Default)]
pub struct Graph {
    pub commits: Vec<Commit>,
    pub nodes: Vec<Node>,
    /// How many lanes the graph is wide.
    pub lanes: usize,
}

impl Graph {
    pub fn new(commits: Vec<Commit>) -> Graph {
        let nodes = layout(&commits);
        let lanes = nodes.iter().flat_map(|n| n.edges.iter().map(|e| e.to).chain([n.lane])).max().map_or(0, |l| l + 1);
        Graph { commits, nodes, lanes }
    }
}

/// The first lane nobody is waiting in, adding one when all are taken.
fn free_lane(lanes: &mut Vec<Option<String>>) -> usize {
    lanes.iter().position(Option::is_none).unwrap_or_else(|| {
        lanes.push(None);
        lanes.len() - 1
    })
}

/// Give every commit a lane. A lane holds the hash of the commit it is
/// waiting for: a commit takes the lane a child reserved for it (or a
/// free one, when it is a branch's tip), then reserves its own lane for
/// its first parent and another for each further one (a merge).
fn layout(commits: &[Commit]) -> Vec<Node> {
    let rows: HashMap<&str, usize> = commits.iter().enumerate().map(|(i, c)| (c.hash.as_str(), i)).collect();
    let mut lanes: Vec<Option<String>> = Vec::new();
    let mut nodes = Vec::with_capacity(commits.len());
    for commit in commits {
        let waiting = |lanes: &[Option<String>], hash: &str| lanes.iter().position(|l| l.as_deref() == Some(hash));
        let lane = match waiting(&lanes, &commit.hash) {
            Some(l) => l,
            None => free_lane(&mut lanes),
        };
        // Other children may have reserved lanes for this commit too:
        // they all end here.
        for slot in lanes.iter_mut().filter(|l| l.as_deref() == Some(commit.hash.as_str())) {
            *slot = None;
        }
        let mut edges = Vec::new();
        for (i, parent) in commit.parents.iter().enumerate() {
            // A parent past the end of the list has no row to draw to.
            let Some(&row) = rows.get(parent.as_str()) else { continue };
            let to = match waiting(&lanes, parent) {
                Some(l) => l,
                None if i == 0 => lane,
                None => free_lane(&mut lanes),
            };
            lanes[to] = Some(parent.clone());
            edges.push(Edge { from: lane, to, row });
        }
        nodes.push(Node { lane, edges });
    }
    nodes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(hash: &str, parents: &[&str]) -> Commit {
        Commit { hash: hash.into(), parents: parents.iter().map(|p| p.to_string()).collect(), ..Commit::default() }
    }

    #[test]
    fn a_straight_line_stays_in_one_lane() {
        let g = Graph::new(vec![commit("c", &["b"]), commit("b", &["a"]), commit("a", &[])]);
        assert_eq!(g.lanes, 1);
        assert_eq!(g.nodes[0], Node { lane: 0, edges: vec![Edge { from: 0, to: 0, row: 1 }] });
        assert!(g.nodes[2].edges.is_empty(), "the first commit has no parent");
    }

    #[test]
    fn a_merge_opens_a_lane_and_the_fork_closes_it() {
        // m merges side (s) into main (b); both come from a.
        let g = Graph::new(vec![commit("m", &["b", "s"]), commit("b", &["a"]), commit("s", &["a"]), commit("a", &[])]);
        assert_eq!(g.lanes, 2);
        assert_eq!(g.nodes[0].edges, [Edge { from: 0, to: 0, row: 1 }, Edge { from: 0, to: 1, row: 2 }]);
        assert_eq!((g.nodes[1].lane, g.nodes[2].lane, g.nodes[3].lane), (0, 1, 0));
        // The side branch's line runs back into the lane its parent is in.
        assert_eq!(g.nodes[2].edges, [Edge { from: 1, to: 0, row: 3 }]);
        // Both lanes are free again once `a` is placed: a new tip reuses one.
        let again = Graph::new(vec![commit("m", &["b", "s"]), commit("b", &["a"]), commit("s", &["a"]), commit("a", &[]), commit("z", &[])]);
        assert_eq!(again.nodes[4].lane, 0);
    }

    #[test]
    fn a_parent_off_the_end_draws_no_line() {
        let g = Graph::new(vec![commit("b", &["a"])]);
        assert!(g.nodes[0].edges.is_empty());
        assert_eq!(Graph::new(Vec::new()).lanes, 0);
    }
}
