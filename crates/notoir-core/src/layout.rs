//! Editor groups and the split layout tree
//! (docs/specs/03-editor-views-groups-and-splits.md).

use std::collections::BTreeMap;

use crate::ids::{GroupId, ViewId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// Children laid out left → right.
    Horizontal,
    /// Children laid out top → bottom.
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    pub fn axis(self) -> Axis {
        match self {
            Direction::Left | Direction::Right => Axis::Horizontal,
            Direction::Up | Direction::Down => Axis::Vertical,
        }
    }

    /// Whether the new group goes before the target along the axis.
    pub fn before(self) -> bool {
        matches!(self, Direction::Left | Direction::Up)
    }
}

/// A tab strip with its active tab and activation order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorGroup {
    pub id: GroupId,
    pub tabs: Vec<ViewId>,
    pub active: Option<ViewId>,
    /// Most recently activated first.
    pub mru: Vec<ViewId>,
}

impl EditorGroup {
    pub fn new(id: GroupId) -> Self {
        Self {
            id,
            tabs: Vec::new(),
            active: None,
            mru: Vec::new(),
        }
    }

    pub fn contains(&self, view: ViewId) -> bool {
        self.tabs.contains(&view)
    }

    pub fn insert(&mut self, view: ViewId, index: usize) {
        let index = index.min(self.tabs.len());
        self.tabs.insert(index, view);
    }

    pub fn activate(&mut self, view: ViewId) {
        if self.contains(view) {
            self.active = Some(view);
            self.mru.retain(|v| *v != view);
            self.mru.insert(0, view);
        }
    }

    /// Removes a tab; the previous MRU tab becomes active.
    pub fn remove(&mut self, view: ViewId) -> bool {
        let Some(pos) = self.tabs.iter().position(|v| *v == view) else {
            return false;
        };
        self.tabs.remove(pos);
        self.mru.retain(|v| *v != view);
        if self.active == Some(view) {
            self.active = self
                .mru
                .first()
                .copied()
                .or_else(|| self.tabs.get(pos.saturating_sub(1)).copied());
            if let Some(next) = self.active {
                self.activate(next);
            }
        }
        true
    }

    pub fn index_of(&self, view: ViewId) -> Option<usize> {
        self.tabs.iter().position(|v| *v == view)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum LayoutNode {
    Group(GroupId),
    /// Fractions sum to 1.0.
    Split {
        axis: Axis,
        children: Vec<(LayoutNode, f32)>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayoutTree {
    pub root: LayoutNode,
    pub groups: BTreeMap<GroupId, EditorGroup>,
    pub active_group: GroupId,
    /// Group temporarily maximized (spec 03 §Maximize); the app saves and
    /// restores panel visibility around it.
    pub maximized: Option<GroupId>,
}

/// Index path from the root to a node.
type Path = Vec<usize>;

impl LayoutTree {
    pub fn new(first: GroupId) -> Self {
        let mut groups = BTreeMap::new();
        groups.insert(first, EditorGroup::new(first));
        Self {
            root: LayoutNode::Group(first),
            groups,
            active_group: first,
            maximized: None,
        }
    }

    pub fn group(&self, id: GroupId) -> Option<&EditorGroup> {
        self.groups.get(&id)
    }

    pub fn group_mut(&mut self, id: GroupId) -> Option<&mut EditorGroup> {
        self.groups.get_mut(&id)
    }

    pub fn active(&self) -> &EditorGroup {
        &self.groups[&self.active_group]
    }

    pub fn group_of(&self, view: ViewId) -> Option<GroupId> {
        self.groups
            .values()
            .find(|g| g.contains(view))
            .map(|g| g.id)
    }

    /// Groups in depth-first (visual reading) order.
    pub fn groups_in_order(&self) -> Vec<GroupId> {
        fn walk(n: &LayoutNode, out: &mut Vec<GroupId>) {
            match n {
                LayoutNode::Group(g) => out.push(*g),
                LayoutNode::Split { children, .. } => {
                    children.iter().for_each(|(c, _)| walk(c, out))
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.root, &mut out);
        out
    }

    /// Next/previous group in layout order, cycling.
    pub fn neighbor(&self, of: GroupId, forward: bool) -> GroupId {
        let order = self.groups_in_order();
        let i = order.iter().position(|g| *g == of).unwrap_or(0);
        let n = order.len();
        order[if forward {
            (i + 1) % n
        } else {
            (i + n - 1) % n
        }]
    }

    /// Adds group `new` adjacent to `target` in `dir`. Returns false if
    /// `target` does not exist or `new` already does.
    pub fn split(&mut self, target: GroupId, dir: Direction, new: GroupId) -> bool {
        if self.groups.contains_key(&new) {
            return false;
        }
        let Some(path) = self.path_to(target) else {
            return false;
        };
        let leaf = LayoutNode::Group(new);
        let parent_matches = path
            .split_last()
            .and_then(|(_, parent)| match self.node_at(parent) {
                LayoutNode::Split { axis, .. } => Some(*axis == dir.axis()),
                LayoutNode::Group(_) => None,
            })
            .unwrap_or(false);
        if parent_matches {
            let (&idx, parent) = path.split_last().expect("non-root");
            if let LayoutNode::Split { children, .. } = self.node_at_mut(parent) {
                let half = children[idx].1 / 2.0;
                children[idx].1 = half;
                let at = if dir.before() { idx } else { idx + 1 };
                children.insert(at, (leaf, half));
            }
        } else {
            let node = self.node_at_mut(&path);
            let old = std::mem::replace(node, LayoutNode::Group(target));
            let pair = if dir.before() {
                [leaf, old]
            } else {
                [old, leaf]
            };
            *node = LayoutNode::Split {
                axis: dir.axis(),
                children: pair.into_iter().map(|n| (n, 0.5)).collect(),
            };
        }
        self.groups.insert(new, EditorGroup::new(new));
        true
    }

    /// Removes a group from the tree (its tabs must have been moved/closed by
    /// the caller). The last group can never be removed.
    pub fn remove_group(&mut self, id: GroupId) -> bool {
        if self.groups.len() <= 1 || !self.groups.contains_key(&id) {
            return false;
        }
        let Some(path) = self.path_to(id) else {
            return false;
        };
        let (&idx, parent) = path.split_last().expect("not the only group");
        if let LayoutNode::Split { children, .. } = self.node_at_mut(parent) {
            children.remove(idx);
        }
        self.groups.remove(&id);
        normalize(&mut self.root);
        if self.active_group == id {
            self.active_group = self.groups_in_order()[0];
        }
        if self.maximized == Some(id) {
            self.maximized = None;
        }
        true
    }

    /// Moves the divider before child `index + 1` of the split at `path` by
    /// `delta` (fraction of that split), keeping every child ≥ `min_fraction`.
    pub fn resize(&mut self, path: &[usize], index: usize, delta: f32, min_fraction: f32) {
        if let LayoutNode::Split { children, .. } = self.node_at_mut(path) {
            if index + 1 >= children.len() {
                return;
            }
            let (a, b) = (children[index].1, children[index + 1].1);
            if a + b < 2.0 * min_fraction {
                return;
            }
            let delta = delta.clamp(min_fraction - a, b - min_fraction);
            children[index].1 = a + delta;
            children[index + 1].1 = b - delta;
        }
    }

    fn path_to(&self, id: GroupId) -> Option<Path> {
        fn find(n: &LayoutNode, id: GroupId, path: &mut Path) -> bool {
            match n {
                LayoutNode::Group(g) => *g == id,
                LayoutNode::Split { children, .. } => {
                    for (i, (c, _)) in children.iter().enumerate() {
                        path.push(i);
                        if find(c, id, path) {
                            return true;
                        }
                        path.pop();
                    }
                    false
                }
            }
        }
        let mut path = Vec::new();
        find(&self.root, id, &mut path).then_some(path)
    }

    fn node_at(&self, path: &[usize]) -> &LayoutNode {
        let mut n = &self.root;
        for &i in path {
            if let LayoutNode::Split { children, .. } = n {
                n = &children[i].0;
            }
        }
        n
    }

    fn node_at_mut(&mut self, path: &[usize]) -> &mut LayoutNode {
        let mut n = &mut self.root;
        for &i in path {
            if let LayoutNode::Split { children, .. } = n {
                n = &mut children[i].0;
            }
        }
        n
    }

    /// Checks structural invariants (tests and debug assertions).
    pub fn check(&self) -> Result<(), String> {
        fn walk(
            n: &LayoutNode,
            parent_axis: Option<Axis>,
            seen: &mut Vec<GroupId>,
        ) -> Result<(), String> {
            match n {
                LayoutNode::Group(g) => {
                    seen.push(*g);
                    Ok(())
                }
                LayoutNode::Split { axis, children } => {
                    if children.len() < 2 {
                        return Err("split with fewer than 2 children".into());
                    }
                    if parent_axis == Some(*axis) {
                        return Err("nested split on the same axis".into());
                    }
                    let sum: f32 = children.iter().map(|(_, f)| f).sum();
                    if (sum - 1.0).abs() > 1e-3 {
                        return Err(format!("fractions sum to {sum}"));
                    }
                    children
                        .iter()
                        .try_for_each(|(c, _)| walk(c, Some(*axis), seen))
                }
            }
        }
        let mut seen = Vec::new();
        walk(&self.root, None, &mut seen)?;
        let mut sorted = seen.clone();
        sorted.sort();
        sorted.dedup();
        if sorted.len() != seen.len() {
            return Err("group appears twice".into());
        }
        if sorted != self.groups.keys().copied().collect::<Vec<_>>() {
            return Err("tree and group map disagree".into());
        }
        if !self.groups.contains_key(&self.active_group) {
            return Err("active group missing".into());
        }
        Ok(())
    }
}

/// Collapses single-child splits, flattens same-axis nesting and renormalizes
/// fractions.
fn normalize(node: &mut LayoutNode) {
    let LayoutNode::Split { axis, children } = node else {
        return;
    };
    let axis = *axis;
    for (c, _) in children.iter_mut() {
        normalize(c);
    }
    let mut flat = Vec::with_capacity(children.len());
    for (child, f) in children.drain(..) {
        match child {
            LayoutNode::Split {
                axis: a,
                children: inner,
            } if a == axis => {
                flat.extend(inner.into_iter().map(|(c, g)| (c, g * f)));
            }
            other => flat.push((other, f)),
        }
    }
    let sum: f32 = flat.iter().map(|(_, f)| f).sum();
    let count = flat.len() as f32;
    for (_, f) in &mut flat {
        *f = if sum > 0.0 { *f / sum } else { 1.0 / count };
    }
    if flat.len() == 1 {
        *node = flat.pop().expect("one child").0;
    } else {
        *children = flat;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn g(n: u64) -> GroupId {
        GroupId(n)
    }

    // spec 03: "Nested split layout"
    #[test]
    fn split_right_then_down_builds_nested_layout() {
        let mut t = LayoutTree::new(g(1));
        assert!(t.split(g(1), Direction::Down, g(2)));
        assert!(t.split(g(2), Direction::Right, g(3)));
        t.check().unwrap();
        let LayoutNode::Split { axis, children } = &t.root else {
            panic!()
        };
        assert_eq!(*axis, Axis::Vertical);
        assert!(matches!(
            &children[1].0,
            LayoutNode::Split {
                axis: Axis::Horizontal,
                ..
            }
        ));
        assert_eq!(t.groups_in_order(), vec![g(1), g(2), g(3)]);
    }

    #[test]
    fn same_axis_split_inserts_sibling() {
        let mut t = LayoutTree::new(g(1));
        t.split(g(1), Direction::Right, g(2));
        t.split(g(1), Direction::Left, g(3));
        t.check().unwrap();
        assert_eq!(t.groups_in_order(), vec![g(3), g(1), g(2)]);
        let LayoutNode::Split { children, .. } = &t.root else {
            panic!()
        };
        assert_eq!(children.len(), 3);
    }

    #[test]
    fn removing_collapses_and_flattens() {
        let mut t = LayoutTree::new(g(1));
        t.split(g(1), Direction::Right, g(2));
        t.split(g(2), Direction::Down, g(3));
        t.split(g(3), Direction::Right, g(4));
        t.check().unwrap();
        // Removing 2 leaves the vertical split with one child (a horizontal
        // split of 3,4) which must flatten into the root horizontal split.
        assert!(t.remove_group(g(2)));
        t.check().unwrap();
        assert_eq!(t.groups_in_order(), vec![g(1), g(3), g(4)]);
        assert!(t.remove_group(g(3)) && t.remove_group(g(4)));
        assert_eq!(t.root, LayoutNode::Group(g(1)));
        assert!(!t.remove_group(g(1)), "last group stays");
    }

    #[test]
    fn group_mru_activation_on_close() {
        let mut grp = EditorGroup::new(g(1));
        for v in 1..=3 {
            grp.insert(ViewId(v), 9);
            grp.activate(ViewId(v));
        }
        grp.activate(ViewId(1));
        grp.remove(ViewId(1));
        assert_eq!(grp.active, Some(ViewId(3)));
    }

    #[test]
    fn resize_respects_minimum() {
        let mut t = LayoutTree::new(g(1));
        t.split(g(1), Direction::Right, g(2));
        t.resize(&[], 0, 0.9, 0.1);
        let LayoutNode::Split { children, .. } = &t.root else {
            panic!()
        };
        assert!((children[1].1 - 0.1).abs() < 1e-5);
        t.check().unwrap();
    }

    #[derive(Clone, Debug)]
    enum Op {
        Split(usize, u8),
        Remove(usize),
    }

    proptest! {
        #[test]
        fn random_operations_keep_invariants(ops in prop::collection::vec(
            prop_oneof![
                (0..8usize, 0..4u8).prop_map(|(i, d)| Op::Split(i, d)),
                (0..8usize).prop_map(Op::Remove),
            ], 0..40)) {
            let mut t = LayoutTree::new(g(1));
            let mut next = 2;
            for op in ops {
                let order = t.groups_in_order();
                match op {
                    Op::Split(i, d) => {
                        let dir = [Direction::Left, Direction::Right, Direction::Up, Direction::Down][d as usize];
                        t.split(order[i % order.len()], dir, g(next));
                        next += 1;
                    }
                    Op::Remove(i) => {
                        t.remove_group(order[i % order.len()]);
                    }
                }
                prop_assert_eq!(t.check(), Ok(()));
            }
        }
    }
}
