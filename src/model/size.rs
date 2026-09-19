// Copyright The Glide Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

use core::fmt::Debug;
use std::collections::HashMap;
use std::mem;

use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use serde::{Deserialize, Serialize};

use super::layout_tree::TreeEvent;
use super::selection::Selection;
use super::tree::{NodeId, NodeMap};
use crate::actor::app::WindowId;
use crate::config::Config;
use crate::sys::geometry::{CGRectExt, Round};

#[derive(Default, Serialize, Deserialize)]
pub struct Size {
    info: slotmap::SecondaryMap<NodeId, LayoutInfo>,
    /// Size share locks by window, as a fraction of the tiled area.
    ///
    /// A lock freezes the share of the screen its window's estate holds,
    /// regardless of the weights around it. Kept out of the serialized layout
    /// so locks only last for the session.
    #[serde(skip)]
    locks: HashMap<WindowId, f64>,
}

#[allow(unused)]
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainerKind {
    #[default]
    Horizontal,
    Vertical,
    Tabbed,
    Stacked,
}

impl ContainerKind {
    pub fn from(orientation: Orientation) -> Self {
        match orientation {
            Orientation::Horizontal => ContainerKind::Horizontal,
            Orientation::Vertical => ContainerKind::Vertical,
        }
    }

    pub fn group(orientation: Orientation) -> Self {
        match orientation {
            Orientation::Horizontal => ContainerKind::Tabbed,
            Orientation::Vertical => ContainerKind::Stacked,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    Horizontal,
    Vertical,
}

impl ContainerKind {
    pub fn orientation(self) -> Orientation {
        use ContainerKind::*;
        match self {
            Horizontal | Tabbed => Orientation::Horizontal,
            Vertical | Stacked => Orientation::Vertical,
        }
    }

    pub fn is_group(self) -> bool {
        use ContainerKind::*;
        match self {
            Stacked | Tabbed => true,
            _ => false,
        }
    }

    /// Returns the kind with the opposite orientation.
    pub fn flip(self) -> Self {
        use ContainerKind::*;
        match self {
            Horizontal => Vertical,
            Vertical => Horizontal,
            Tabbed => Stacked,
            Stacked => Tabbed,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    pub(super) fn orientation(self) -> Orientation {
        use Direction::*;
        match self {
            Left | Right => Orientation::Horizontal,
            Up | Down => Orientation::Vertical,
        }
    }
}

#[derive(Debug, Clone)]
pub struct GroupBarInfo {
    pub node_id: NodeId,
    pub container_kind: ContainerKind,
    /// Total number of windows in the group
    pub total_count: usize,
    /// Index of the currently selected window
    pub selected_index: usize,
    /// Whether this group should be visible
    pub is_visible: bool,
    /// Whether this group is in the selection path
    pub is_selected: bool,
    /// Frame reserved for the indicator bar.
    pub indicator_frame: CGRect,
    /// Whether this bar is on top of other windows in the layout.
    ///
    /// Always set to true by us; higher layers set to false if there are
    /// floating windows.
    pub is_on_top: bool,
}

// TODO:
//
// It'd be much easier to only move specific edges if we keep the min edge
// of each child (relative to the parent, from 0 to 1). Then we just need
// to adjust this edge, and preserve the invariant that no edge is greater
// than the following edge.
//
// Calculating the size of a single node is easy and just needs to look at the
// next sibling.
//
// Proportional changes would no longer happen by default, but should still be
// relatively easy. Just keep a count of children, and we can adjust each child's
// size in a single scan.
//
// This seems *way* simpler than trying to fix up a proportionate representation
// to create a single edge change.
//
// Actually, on second thought, this would still create proportional resizes of
// children. To prevent that we would need the edges to be absolute (relative
// to the root) and traverse *recursively* when one is modified, fixing up any
// edges that violate our invariant.
//
// This might still be overall simpler than the resize logic would need to be
// for the proportionate case, but it feels more like we are distributing the
// complexity rather than reducing it.

#[derive(Default, Debug, Serialize, Deserialize, Clone)]
struct LayoutInfo {
    /// The share of the parent's size taken up by this node; 1.0 by default.
    size: f32,
    /// The total size of all children.
    total: f32,
    /// The orientation of this node. Not used for leaf nodes.
    kind: ContainerKind,
    /// The last ungrouped layout of this node.
    last_ungrouped_kind: ContainerKind,
    /// Whether the node is fullscreen.
    #[serde(default)]
    is_fullscreen: bool,
}

/// The share of the tiled area an unlocked window is guaranteed when size
/// share locks would otherwise squeeze it out.
const MIN_UNLOCKED_SHARE: f64 = 0.1;

/// The target share of the tiled area for every node under a layout root,
/// derived from size share locks and the layout's weights.
pub(super) struct Masses {
    mass: HashMap<NodeId, f64>,
    locked: HashMap<NodeId, f64>,
}

impl Masses {
    /// The share of the tiled area the node's subtree holds.
    pub(super) fn total(&self, node: NodeId) -> f64 {
        self.mass.get(&node).copied().unwrap_or(0.0)
    }

    /// The part of [`Self::total`] that comes from locked estates.
    pub(super) fn locked(&self, node: NodeId) -> f64 {
        self.locked.get(&node).copied().unwrap_or(0.0)
    }
}

/// A subtree that owns a rectangle of the layout: a window, or a group whose
/// windows share one rectangle.
struct EstateUnit {
    node: NodeId,
    /// The share of the tiled area the unit would hold without locks.
    share: f64,
    lock: Option<f64>,
}

impl Size {
    pub(super) fn handle_event(&mut self, map: &NodeMap, event: TreeEvent) {
        match event {
            TreeEvent::AddedToForest(node) => {
                self.info.insert(node, LayoutInfo::default());
            }
            TreeEvent::AddedToParent(node) => {
                let parent = node.parent(map).unwrap();
                self.info[node].size = 1.0;
                self.info[parent].total += 1.0;
            }
            TreeEvent::Copied { src, dest } => {
                self.info.insert(dest, self.info[src].clone());
            }
            TreeEvent::RemovingFromParent(node) => {
                self.info[node.parent(map).unwrap()].total -= self.info[node].size;
            }
            TreeEvent::RemovedFromForest(node) => {
                self.info.remove(node);
            }
        }
    }

    pub(super) fn assume_size_of(&mut self, new: NodeId, old: NodeId, map: &NodeMap) {
        assert_eq!(new.parent(map), old.parent(map));
        let parent = new.parent(map).unwrap();
        self.info[parent].total -= self.info[new].size;
        self.info[new].size = mem::replace(&mut self.info[old].size, 0.0);
    }

    pub(super) fn set_kind(&mut self, node: NodeId, kind: ContainerKind) {
        self.info[node].kind = kind;
        if !kind.is_group() {
            self.info[node].last_ungrouped_kind = kind;
        }
    }

    pub(super) fn kind(&self, node: NodeId) -> ContainerKind {
        self.info[node].kind
    }

    pub(super) fn last_ungrouped_kind(&self, node: NodeId) -> ContainerKind {
        self.info[node].last_ungrouped_kind
    }

    pub(super) fn proportion(&self, map: &NodeMap, node: NodeId) -> Option<f64> {
        let Some(parent) = node.parent(map) else { return None };
        Some(f64::from(self.info[node].size) / f64::from(self.info[parent].total))
    }

    pub(super) fn total(&self, node: NodeId) -> f64 {
        f64::from(self.info[node].total)
    }

    pub(super) fn weight(&self, node: NodeId) -> f32 {
        self.info[node].size
    }

    pub(super) fn take_share(&mut self, map: &NodeMap, node: NodeId, from: NodeId, share: f32) {
        assert_eq!(node.parent(map), from.parent(map));
        let share = share.min(self.info[from].size);
        let share = share.max(-self.info[node].size);
        self.info[from].size -= share;
        self.info[node].size += share;
    }

    pub(super) fn set_weight(&mut self, node: NodeId, weight: f32, map: &NodeMap) {
        let old = self.info[node].size;
        self.info[node].size = weight;
        if let Some(parent) = node.parent(map) {
            self.info[parent].total += weight - old;
        }
    }

    /// Swaps the weights of two nodes, so their sizes follow them when they
    /// swap positions in the tree.
    pub(super) fn swap_weights(&mut self, node_a: NodeId, node_b: NodeId) {
        let weight_a = self.info[node_a].size;
        let weight_b = self.info[node_b].size;
        self.info[node_a].size = weight_b;
        self.info[node_b].size = weight_a;
        // Parent totals don't change since we're just swapping.
    }

    pub(super) fn set_fullscreen(&mut self, node: NodeId, is_fullscreen: bool) {
        self.info[node].is_fullscreen = is_fullscreen;
    }

    pub(super) fn is_fullscreen(&mut self, node: NodeId) -> bool {
        self.info[node].is_fullscreen
    }

    pub(super) fn set_lock(&mut self, wid: WindowId, share: f64) {
        self.locks.insert(wid, share);
    }

    pub(super) fn clear_lock(&mut self, wid: WindowId) -> bool {
        self.locks.remove(&wid).is_some()
    }

    pub(super) fn clear_locks(&mut self) {
        self.locks.clear();
    }

    pub(super) fn retain_locks(&mut self, mut filter: impl FnMut(WindowId) -> bool) {
        self.locks.retain(|&wid, _| filter(wid));
    }

    pub(super) fn lock(&self, wid: WindowId) -> Option<f64> {
        self.locks.get(&wid).copied()
    }

    /// Computes the target share of the tiled area for every node under
    /// `root`, based on locked windows and the layout's weights.
    ///
    /// Returns `None` when no lock applies, in which case the stored weights
    /// are used as they are.
    pub(super) fn masses(
        &self,
        map: &NodeMap,
        window: &super::window::Window,
        root: NodeId,
    ) -> Option<Masses> {
        if self.locks.is_empty() {
            return None;
        }
        let mut units = Vec::new();
        self.estate_units(map, window, root, 1.0, &mut units);
        let locked_count = units.iter().filter(|unit| unit.lock.is_some()).count();
        if locked_count == 0 {
            return None;
        }

        let unlocked_count = units.len() - locked_count;
        let locked_total: f64 = units.iter().filter_map(|unit| unit.lock).sum();
        // Locked windows keep their share of the screen, except when doing so
        // would leave unlocked windows with less than a usable share of it.
        // The shortfall comes out of the locks, proportionally.
        let min_unlocked_share = MIN_UNLOCKED_SHARE.min(1.0 / units.len() as f64);
        let min_unlocked_total = min_unlocked_share * unlocked_count as f64;
        let free = (1.0 - locked_total).max(0.0);
        let unlocked_total = if unlocked_count == 0 {
            0.0
        } else {
            free.max(min_unlocked_total).min(1.0)
        };
        let locked_scale = if locked_total > 0.0 {
            ((1.0 - unlocked_total) / locked_total).min(1.0)
        } else {
            1.0
        };

        let share_unlocked: f64 =
            units.iter().filter(|unit| unit.lock.is_none()).map(|unit| unit.share).sum();
        let mut masses = Masses {
            mass: HashMap::new(),
            locked: HashMap::new(),
        };
        for unit in &units {
            let (mass, locked) = match unit.lock {
                Some(share) => {
                    let mass = share * locked_scale;
                    (mass, mass)
                }
                None => {
                    let mass = if share_unlocked > 0.0 {
                        unlocked_total * unit.share / share_unlocked
                    } else {
                        0.0
                    };
                    (mass, 0.0)
                }
            };
            masses.mass.insert(unit.node, mass);
            masses.locked.insert(unit.node, locked);
        }
        fill_container_masses(map, root, &mut masses);
        Some(masses)
    }

    /// Collects the leaves and groups under `node`, which are the largest
    /// subtrees that own a rectangle of the layout.
    fn estate_units(
        &self,
        map: &NodeMap,
        window: &super::window::Window,
        node: NodeId,
        share: f64,
        units: &mut Vec<EstateUnit>,
    ) {
        if let Some(wid) = window.at(node) {
            units.push(EstateUnit {
                node,
                share,
                lock: self.locks.get(&wid).copied(),
            });
            return;
        }
        if self.info[node].kind.is_group() {
            // A group shows one rectangle for all its windows, so a lock on
            // any window in it applies to the group as a whole.
            let lock = node
                .traverse_preorder(map)
                .filter(|&n| n != node)
                .filter_map(|n| window.at(n))
                .filter_map(|wid| self.locks.get(&wid).copied())
                .reduce(f64::max);
            units.push(EstateUnit { node, share, lock });
            return;
        }
        for child in node.children(map) {
            let child_share =
                self.proportion(map, child).filter(|share| share.is_finite()).unwrap_or(0.0);
            self.estate_units(map, window, child, share * child_share, units);
        }
    }

    pub(super) fn debug(&self, node: NodeId, is_container: bool) -> String {
        let info = &self.info[node];
        let fullscreen = if info.is_fullscreen {
            "; fullscreen"
        } else {
            ""
        };
        if is_container {
            format!(
                "{:?} [size {} total={}{fullscreen}]",
                info.kind, info.size, info.total
            )
        } else {
            format!("[size {}{fullscreen}]", info.size)
        }
    }

    pub(super) fn get_sizes(
        &self,
        map: &NodeMap,
        window: &super::window::Window,
        selection: &Selection,
        config: &Config,
        root: NodeId,
        screen: CGRect,
        is_scroll: bool,
    ) -> Vec<(WindowId, CGRect)> {
        let mut sizes = vec![];
        let masses = (!is_scroll).then(|| self.masses(map, window, root)).flatten();
        Visitor {
            map,
            size: self,
            window,
            selection,
            fullscreen_nodes: &[],
            config,
            screen,
            is_scroll,
            masses: masses.as_ref(),
            sizes: &mut sizes,
            groups: None,
            target: None,
            target_rect: None,
        }
        .visit(root, screen);
        sizes
    }

    /// Returns the rect the layout assigns to `node`.
    ///
    /// `node` must be reachable from `root`.
    pub(super) fn get_node_rect(
        &self,
        map: &NodeMap,
        window: &super::window::Window,
        selection: &Selection,
        config: &Config,
        root: NodeId,
        screen: CGRect,
        is_scroll: bool,
        node: NodeId,
    ) -> Option<CGRect> {
        let mut sizes = vec![];
        let masses = (!is_scroll).then(|| self.masses(map, window, root)).flatten();
        Visitor {
            map,
            size: self,
            window,
            selection,
            fullscreen_nodes: &[],
            config,
            screen,
            is_scroll,
            masses: masses.as_ref(),
            sizes: &mut sizes,
            groups: None,
            target: Some(node),
            target_rect: None,
        }
        .visit(root, screen)
    }

    /// The extent, along `parent`'s orientation, that one unit of `parent`'s
    /// total weight is worth in its children's frames.
    ///
    /// The inner gaps between children are subtracted first, since they are not
    /// distributed by weight.
    pub(super) fn pixels_per_weight(
        &self,
        map: &NodeMap,
        parent: NodeId,
        parent_rect: CGRect,
        config: &Config,
        is_scroll_root: bool,
    ) -> f64 {
        let extent = match self.kind(parent).orientation() {
            Orientation::Horizontal => parent_rect.size.width,
            Orientation::Vertical => parent_rect.size.height,
        };
        let total = self.total(parent);
        // Mirrors the inputs `Visitor` gives to `solve_sizes`.
        let available = if is_scroll_root {
            total * extent
        } else {
            extent
        };
        let count = parent.children(map).count() as f64;
        let usable = available - config.settings.inner_gap * (count - 1.0).max(0.0);
        usable / total
    }

    /// The smallest extent `node` can take along `orientation`, accounting for
    /// the minimum sizes observed for the windows inside it.
    ///
    /// A node with no known minima returns zero. Group children share a frame,
    /// so they contribute their maximum extent rather than their sum.
    fn min_extent(
        &self,
        map: &NodeMap,
        window: &super::window::Window,
        config: &Config,
        node: NodeId,
        orientation: Orientation,
    ) -> f64 {
        if self.info[node].is_fullscreen {
            // A fullscreen node takes the whole screen regardless of its slot.
            return 0.0;
        }
        if let Some(wid) = window.at(node) {
            return window.min_size(wid).map_or(0.0, |min_size| match orientation {
                Orientation::Horizontal => min_size.width,
                Orientation::Vertical => min_size.height,
            });
        }

        let children: Vec<NodeId> = node.children(map).collect();
        if children.is_empty() {
            return 0.0;
        }
        let extents: Vec<f64> = children
            .iter()
            .map(|&child| self.min_extent(map, window, config, child, orientation))
            .collect();
        let child_max = extents.iter().copied().fold(0.0, f64::max);

        use ContainerKind::*;
        let info = &self.info[node];
        match info.kind {
            // The indicator bar takes space perpendicular to the container's
            // orientation; along it, the children share the same extent.
            Tabbed | Stacked if info.kind.orientation() != orientation => {
                if config.settings.group_bars.enable {
                    child_max + config.settings.group_bars.thickness
                } else {
                    child_max
                }
            }
            Horizontal | Vertical if info.kind.orientation() == orientation => {
                let gaps = config.settings.inner_gap * (children.len() as f64 - 1.0);
                extents.iter().sum::<f64>() + gaps
            }
            _ => child_max,
        }
    }

    pub(super) fn get_sizes_and_groups(
        &self,
        map: &NodeMap,
        window: &super::window::Window,
        selection: &Selection,
        config: &Config,
        root: NodeId,
        screen: CGRect,
        is_scroll: bool,
    ) -> (Vec<(WindowId, CGRect)>, Vec<GroupBarInfo>) {
        let mut sizes = vec![];
        let mut groups = vec![];
        let fullscreen_nodes = &root
            .traverse_postorder(map)
            .filter(|&node| self.info.get(node).map(|i| i.is_fullscreen).unwrap_or(false))
            .collect::<Vec<_>>();
        let masses = (!is_scroll).then(|| self.masses(map, window, root)).flatten();
        Visitor {
            map,
            size: self,
            window,
            selection,
            fullscreen_nodes,
            config,
            screen,
            is_scroll,
            masses: masses.as_ref(),
            sizes: &mut sizes,
            groups: Some(&mut groups),
            target: None,
            target_rect: None,
        }
        .visit(root, screen);
        (sizes, groups)
    }
}

/// Fills in the mass of every container under `node`, summing the masses of
/// the estate units below it.
fn fill_container_masses(map: &NodeMap, node: NodeId, masses: &mut Masses) {
    if masses.mass.contains_key(&node) {
        return;
    }
    let mut mass = 0.0;
    let mut locked = 0.0;
    for child in node.children(map) {
        fill_container_masses(map, child, masses);
        mass += masses.total(child);
        locked += masses.locked(child);
    }
    masses.mass.insert(node, mass);
    masses.locked.insert(node, locked);
}

struct Visitor<'a, 'out> {
    map: &'a NodeMap,
    size: &'a Size,
    window: &'a super::window::Window,
    selection: &'a Selection,
    fullscreen_nodes: &'a [NodeId],
    config: &'a Config,
    screen: CGRect,
    is_scroll: bool,
    /// Target shares from size share locks. `None` when no lock applies.
    masses: Option<&'a Masses>,
    sizes: &'out mut Vec<(WindowId, CGRect)>,
    groups: Option<&'out mut Vec<GroupBarInfo>>,
    /// If set, the rect assigned to this node is recorded in `target_rect`.
    target: Option<NodeId>,
    target_rect: Option<CGRect>,
}

impl<'a, 'out> Visitor<'a, 'out> {
    /// The weight that decides `child`'s size within its parent.
    fn weight(&self, child: NodeId) -> f64 {
        match self.masses {
            Some(masses) => masses.total(child),
            None => f64::from(self.size.info[child].size),
        }
    }

    fn visit(mut self, root: NodeId, rect: CGRect) -> Option<CGRect> {
        // Usually this should be false, except in the uncommon case where root
        // is fullscreen.
        let parent_visible = self.fullscreen_nodes.contains(&root);
        let rect = rect.inset(self.config.settings.outer_gap);
        // Locked windows can leave part of the tiled area unused. The leftover
        // stays empty instead of stretching the other windows into it, which
        // would defeat the lock.
        let rect = match self.masses {
            Some(masses) if masses.total(root) < 1.0 => {
                let mut rect = rect;
                let free = masses.total(root);
                match self.size.kind(root).orientation() {
                    Orientation::Horizontal => rect.size.width *= free,
                    Orientation::Vertical => rect.size.height *= free,
                }
                rect
            }
            _ => rect,
        };
        self.visit_node(root, rect, true, parent_visible, true);
        self.target_rect
    }

    fn visit_node(
        &mut self,
        node: NodeId,
        rect: CGRect,
        is_in_visibility_path: bool,
        is_parent_visible: bool,
        is_selected: bool,
    ) {
        let info = &self.size.info[node];
        let rect = if info.is_fullscreen {
            self.screen.inset(self.config.settings.outer_gap)
        } else {
            rect
        };

        if self.target == Some(node) {
            self.target_rect = Some(rect);
        }

        if let Some(wid) = self.window.at(node) {
            debug_assert!(
                node.children(self.map).next().is_none(),
                "non-leaf node with window id"
            );
            self.sizes.push((wid, rect));
            return;
        }

        use ContainerKind::*;
        match info.kind {
            Tabbed | Stacked => {
                let (group_frame, indicator_frame) = if self.config.settings.group_bars.enable {
                    size_with_group_indicator(rect, info.kind, &self.config.settings.group_bars)
                } else {
                    (rect, CGRect::ZERO)
                };

                // Slightly janky visibility computation: If a node is fullscreen
                // only its descendants can be considered visible. If multiple
                // nodes are fullscreen we don't attempt to handle it well.
                let is_visible = is_in_visibility_path
                    && (is_parent_visible
                        || self.fullscreen_nodes.is_empty()
                        || self.fullscreen_nodes.contains(&node));

                let selected_child = self.selection.last_selection(self.map, node);
                let mut selected_index = 0;
                let mut num_children = 0;
                for (index, child) in node.children(self.map).enumerate() {
                    let selected = selected_child == Some(child);
                    if selected {
                        selected_index = index;
                    }
                    num_children += 1;

                    self.visit_node(
                        child,
                        group_frame,
                        is_in_visibility_path && selected,
                        is_visible,
                        is_selected && selected,
                    );
                }

                if let Some(groups) = self.groups.as_deref_mut() {
                    groups.push(GroupBarInfo {
                        node_id: node,
                        container_kind: info.kind,
                        indicator_frame,
                        total_count: num_children,
                        selected_index,
                        is_visible,
                        is_selected,
                        is_on_top: true,
                    });
                }
            }
            Horizontal => {
                let inner_gap = self.config.settings.inner_gap;
                let local_selection = self.selection.local_selection(self.map, node);
                let children: Vec<NodeId> = node.children(self.map).collect();

                let aspect_max_width = if children.len() == 1 {
                    self.config
                        .settings
                        .experimental
                        .scroll
                        .aspect_ratio()
                        .map(|ar| rect.size.height * (ar.width / ar.height))
                } else {
                    None
                };

                let inputs: Vec<super::scroll_constraints::WindowInput> = children
                    .iter()
                    .map(|&child| super::scroll_constraints::WindowInput {
                        weight: self.weight(child),
                        min_size: if self.is_scroll {
                            super::scroll_constraints::MIN_WINDOW_SIZE
                        } else {
                            self.size
                                .min_extent(
                                    self.map,
                                    self.window,
                                    self.config,
                                    child,
                                    Orientation::Horizontal,
                                )
                                .max(1.0)
                        },
                        max_size: aspect_max_width,
                        fixed_size: None,
                    })
                    .collect();

                let total_weight: f64 = inputs.iter().map(|i| i.weight).sum();
                let virtual_width = if self.is_scroll {
                    total_weight * rect.size.width
                } else {
                    rect.size.width
                };
                let outputs =
                    super::scroll_constraints::solve_sizes(&inputs, virtual_width, inner_gap);

                let mut x = rect.origin.x;
                for (&child, output) in children.iter().zip(outputs.iter()) {
                    let rect = CGRect {
                        origin: CGPoint { x, y: rect.origin.y },
                        size: CGSize {
                            width: output.size,
                            height: rect.size.height,
                        },
                    }
                    .round();
                    self.visit_node(
                        child,
                        rect,
                        is_in_visibility_path,
                        is_parent_visible,
                        is_selected && local_selection == Some(child),
                    );
                    x = rect.max().x + inner_gap;
                }
            }
            Vertical => {
                let inner_gap = self.config.settings.inner_gap;
                let local_selection = self.selection.local_selection(self.map, node);
                let children: Vec<NodeId> = node.children(self.map).collect();

                let inputs: Vec<super::scroll_constraints::WindowInput> = children
                    .iter()
                    .map(|&child| super::scroll_constraints::WindowInput {
                        weight: self.weight(child),
                        min_size: if self.is_scroll {
                            super::scroll_constraints::MIN_WINDOW_SIZE
                        } else {
                            self.size
                                .min_extent(
                                    self.map,
                                    self.window,
                                    self.config,
                                    child,
                                    Orientation::Vertical,
                                )
                                .max(1.0)
                        },
                        max_size: None,
                        fixed_size: None,
                    })
                    .collect();

                let outputs =
                    super::scroll_constraints::solve_sizes(&inputs, rect.size.height, inner_gap);

                let mut y = rect.origin.y;
                for (&child, output) in children.iter().zip(outputs.iter()) {
                    let rect = CGRect {
                        origin: CGPoint { x: rect.origin.x, y },
                        size: CGSize {
                            width: rect.size.width,
                            height: output.size,
                        },
                    }
                    .round();
                    self.visit_node(
                        child,
                        rect,
                        is_in_visibility_path,
                        is_parent_visible,
                        is_selected && local_selection == Some(child),
                    );
                    y = rect.max().y + inner_gap;
                }
            }
        }
    }
}

/// Calculate frames for group and indicator, reserving space for the indicator
fn size_with_group_indicator(
    rect: CGRect,
    container_kind: ContainerKind,
    config: &crate::config::GroupBars,
) -> (CGRect, CGRect) {
    use crate::config::{HorizontalPlacement, VerticalPlacement};

    let thickness = config.thickness;

    match container_kind {
        ContainerKind::Tabbed => {
            // Horizontal indicator
            match config.horizontal_placement {
                HorizontalPlacement::Top => {
                    let group_frame = CGRect {
                        origin: CGPoint {
                            x: rect.origin.x,
                            y: rect.origin.y + thickness,
                        },
                        size: CGSize {
                            width: rect.size.width,
                            height: rect.size.height - thickness,
                        },
                    };
                    let indicator_frame = CGRect {
                        origin: rect.origin,
                        size: CGSize {
                            width: rect.size.width,
                            height: thickness,
                        },
                    };
                    (group_frame, indicator_frame)
                }
                HorizontalPlacement::Bottom => {
                    let group_frame = CGRect {
                        origin: rect.origin,
                        size: CGSize {
                            width: rect.size.width,
                            height: rect.size.height - thickness,
                        },
                    };
                    let indicator_frame = CGRect {
                        origin: CGPoint {
                            x: rect.origin.x,
                            y: rect.origin.y + group_frame.size.height,
                        },
                        size: CGSize {
                            width: rect.size.width,
                            height: thickness,
                        },
                    };
                    (group_frame, indicator_frame)
                }
            }
        }
        ContainerKind::Stacked => {
            // Vertical indicator
            match config.vertical_placement {
                VerticalPlacement::Left => {
                    let group_frame = CGRect {
                        origin: CGPoint {
                            x: rect.origin.x + thickness,
                            y: rect.origin.y,
                        },
                        size: CGSize {
                            width: rect.size.width - thickness,
                            height: rect.size.height,
                        },
                    };
                    let indicator_frame = CGRect {
                        origin: rect.origin,
                        size: CGSize {
                            width: thickness,
                            height: rect.size.height,
                        },
                    };
                    (group_frame, indicator_frame)
                }
                VerticalPlacement::Right => {
                    let group_frame = CGRect {
                        origin: rect.origin,
                        size: CGSize {
                            width: rect.size.width - thickness,
                            height: rect.size.height,
                        },
                    };
                    let indicator_frame = CGRect {
                        origin: CGPoint {
                            x: rect.origin.x + group_frame.size.width,
                            y: rect.origin.y,
                        },
                        size: CGSize {
                            width: thickness,
                            height: rect.size.height,
                        },
                    };
                    (group_frame, indicator_frame)
                }
            }
        }
        _ => (rect, CGRect::ZERO),
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::model::LayoutTree;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> CGRect {
        CGRect::new(
            CGPoint::new(f64::from(x), f64::from(y)),
            CGSize::new(f64::from(w), f64::from(h)),
        )
    }

    #[test]
    fn it_lays_out_windows_proportionally() {
        let mut tree = LayoutTree::new();
        let layout = tree.create_layout();
        let root = tree.root(layout);
        let _a1 = tree.add_window_under(layout, root, WindowId::new(1, 1));
        let a2 = tree.add_container(root, ContainerKind::Vertical);
        let _b1 = tree.add_window_under(layout, a2, WindowId::new(1, 2));
        let _b2 = tree.add_window_under(layout, a2, WindowId::new(1, 3));
        let _a3 = tree.add_window_under(layout, root, WindowId::new(1, 4));

        let screen = rect(0, 0, 3000, 1000);
        let (mut frames, groups) =
            tree.calculate_layout_and_groups(layout, screen, &Config::default());
        frames.sort_by_key(|&(wid, _)| wid);
        assert_eq!(
            frames,
            vec![
                (WindowId::new(1, 1), rect(0, 0, 1000, 1000)),
                (WindowId::new(1, 2), rect(1000, 0, 1000, 500)),
                (WindowId::new(1, 3), rect(1000, 500, 1000, 500)),
                (WindowId::new(1, 4), rect(2000, 0, 1000, 1000)),
            ]
        );
        assert_eq!(groups.len(), 0);
    }

    #[test]
    fn it_collects_group_information_for_tabbed_containers() {
        let mut tree = LayoutTree::new();
        let layout = tree.create_layout();
        let root = tree.root(layout);
        let _a1 = tree.add_window_under(layout, root, WindowId::new(1, 1));

        // Create a tabbed group with 3 windows
        let tabbed_group = tree.add_container(root, ContainerKind::Tabbed);
        let _tab1 = tree.add_window_under(layout, tabbed_group, WindowId::new(2, 1));
        let _tab2 = tree.add_window_under(layout, tabbed_group, WindowId::new(2, 2));
        let _tab3 = tree.add_window_under(layout, tabbed_group, WindowId::new(2, 3));

        let _a3 = tree.add_window_under(layout, root, WindowId::new(3, 1));

        let screen = rect(0, 0, 3000, 1000);
        let config = Config::default();
        let (frames, groups) = tree.calculate_layout_and_groups(layout, screen, &config);

        assert_eq!(frames.len(), 5);
        assert_eq!(groups.len(), 1);

        let group = &groups[0];
        assert_eq!(group.node_id, tabbed_group);
        assert_eq!(group.container_kind, ContainerKind::Tabbed);
        assert_eq!(group.total_count, 3);
        assert_eq!(group.selected_index, 0); // First child selected by default
        assert_eq!(group.is_visible, true); // Root level group is visible
    }

    #[test]
    fn it_collects_group_information_for_stacked_containers() {
        let mut tree = LayoutTree::new();
        let layout = tree.create_layout();
        let root = tree.root(layout);

        // Create a stacked group with 2 windows
        let stacked_group = tree.add_container(root, ContainerKind::Stacked);
        let _child1 = tree.add_window_under(layout, stacked_group, WindowId::new(1, 1));
        let _child2 = tree.add_window_under(layout, stacked_group, WindowId::new(1, 2));
        tree.select(_child2);

        let screen = rect(0, 0, 1000, 1000);
        let config = Config::default();
        let (frames, groups) = tree.calculate_layout_and_groups(layout, screen, &config);

        assert_eq!(frames.len(), 2);
        assert_eq!(groups.len(), 1);

        let group = &groups[0];
        assert_eq!(group.node_id, stacked_group);
        assert_eq!(group.container_kind, ContainerKind::Stacked);
        assert_eq!(group.total_count, 2);
        assert_eq!(group.selected_index, 1);
        assert_eq!(group.is_visible, true);
    }

    #[test]
    fn it_tracks_visibility_for_nested_groups() {
        let mut tree = LayoutTree::new();
        let layout = tree.create_layout();
        let root = tree.root(layout);

        // Create outer tabbed group
        let outer_group = tree.add_container(root, ContainerKind::Tabbed);
        let _outer_tab1 = tree.add_window_under(layout, outer_group, WindowId::new(1, 1));

        // Create inner stacked group as second tab (not selected)
        let inner_group = tree.add_container(outer_group, ContainerKind::Stacked);
        let _inner_stack1 = tree.add_window_under(layout, inner_group, WindowId::new(2, 1));
        let _inner_stack2 = tree.add_window_under(layout, inner_group, WindowId::new(2, 2));

        let screen = rect(0, 0, 1000, 1000);
        let config = Config::default();
        let (frames, groups) = tree.calculate_layout_and_groups(layout, screen, &config);

        assert_eq!(frames.len(), 3);
        assert_eq!(groups.len(), 2);

        // Find groups by kind
        let outer = groups.iter().find(|g| g.container_kind == ContainerKind::Tabbed).unwrap();
        let inner = groups.iter().find(|g| g.container_kind == ContainerKind::Stacked).unwrap();

        // Outer group should be visible
        assert_eq!(outer.is_visible, true);
        assert_eq!(outer.total_count, 2); // window + inner group
        assert_eq!(outer.selected_index, 0); // First tab selected

        // Inner group should not be visible (not the selected tab)
        assert_eq!(inner.is_visible, false);
        assert_eq!(inner.total_count, 2);
    }

    #[test]
    fn it_does_not_show_groups_obscured_by_fullscreen_nodes() {
        let mut tree = LayoutTree::new();
        let layout = tree.create_layout();
        let root = tree.root(layout);

        // Create outer tabbed group
        let outer_group = tree.add_container(root, ContainerKind::Tabbed);
        let _outer_tab1 = tree.add_window_under(layout, outer_group, WindowId::new(1, 1));

        // Create inner stacked group as second tab (selected)
        let inner_group = tree.add_container(outer_group, ContainerKind::Stacked);
        let inner_stack1 = tree.add_window_under(layout, inner_group, WindowId::new(2, 1));
        let _inner_stack2 = tree.add_window_under(layout, inner_group, WindowId::new(2, 2));
        tree.select(inner_stack1);

        let screen = rect(0, 0, 1000, 1000);
        let config = Config::default();

        // If inner_group is fullscreen, only its indicator should be visible.
        tree.set_fullscreen(inner_group, true);
        let (_frames, groups) = tree.calculate_layout_and_groups(layout, screen, &config);
        let outer = groups.iter().find(|g| g.container_kind == ContainerKind::Tabbed).unwrap();
        let inner = groups.iter().find(|g| g.container_kind == ContainerKind::Stacked).unwrap();
        assert_eq!(outer.is_visible, false);
        assert_eq!(inner.is_visible, true);

        // If a window inside inner_group is fullscreen, no indicators should be visible.
        tree.set_fullscreen(inner_group, false);
        tree.set_fullscreen(inner_stack1, true);
        let (_frames, groups) = tree.calculate_layout_and_groups(layout, screen, &config);
        let outer = groups.iter().find(|g| g.container_kind == ContainerKind::Tabbed).unwrap();
        let inner = groups.iter().find(|g| g.container_kind == ContainerKind::Stacked).unwrap();
        assert_eq!(outer.is_visible, false);
        assert_eq!(inner.is_visible, false);

        // If the root is fullscreen for some reason, it behaves as normal.
        tree.set_fullscreen(inner_stack1, false);
        tree.set_fullscreen(root, true);
        let (_frames, groups) = tree.calculate_layout_and_groups(layout, screen, &config);
        let outer = groups.iter().find(|g| g.container_kind == ContainerKind::Tabbed).unwrap();
        let inner = groups.iter().find(|g| g.container_kind == ContainerKind::Stacked).unwrap();
        assert_eq!(outer.is_visible, true);
        assert_eq!(inner.is_visible, true);
    }

    #[test]
    fn it_handles_regular_containers_without_groups() {
        let mut tree = LayoutTree::new();
        let layout = tree.create_layout();
        let root = tree.root(layout);
        let _a1 = tree.add_window_under(layout, root, WindowId::new(1, 1));

        // Create a regular vertical container (not a group)
        let vertical_container = tree.add_container(root, ContainerKind::Vertical);
        let _b1 = tree.add_window_under(layout, vertical_container, WindowId::new(2, 1));
        let _b2 = tree.add_window_under(layout, vertical_container, WindowId::new(2, 2));

        let screen = rect(0, 0, 1000, 1000);
        let config = Config::default();
        let (frames, groups) = tree.calculate_layout_and_groups(layout, screen, &config);

        assert_eq!(frames.len(), 3);
        assert_eq!(groups.len(), 0);
    }

    #[test]
    fn it_reserves_space_for_indicators_when_enabled() {
        let mut tree = LayoutTree::new();
        let layout = tree.create_layout();
        let root = tree.root(layout);

        // Create a tabbed group
        let tabbed_group = tree.add_container(root, ContainerKind::Tabbed);
        let _tab1 = tree.add_window_under(layout, tabbed_group, WindowId::new(1, 1));
        let _tab2 = tree.add_window_under(layout, tabbed_group, WindowId::new(1, 2));

        let screen = rect(0, 0, 1000, 1000);

        // Test with indicators disabled
        let mut config_disabled = Config::default();
        config_disabled.settings.group_bars.enable = false;
        let (frames_disabled, groups_disabled) =
            tree.calculate_layout_and_groups(layout, screen, &config_disabled);

        // Test with indicators enabled
        let mut config_enabled = Config::default();
        config_enabled.settings.group_bars.enable = true;
        config_enabled.settings.group_bars.thickness = 20.0;
        let (frames_enabled, groups_enabled) =
            tree.calculate_layout_and_groups(layout, screen, &config_enabled);

        // Both should have same number of frames and groups
        assert_eq!(frames_disabled.len(), frames_enabled.len());
        assert_eq!(groups_disabled.len(), groups_enabled.len());
        assert_eq!(groups_enabled.len(), 1);

        // When disabled, indicator frame should be zero (no indicator to display)
        let group_disabled = &groups_disabled[0];
        assert_eq!(group_disabled.indicator_frame, rect(0, 0, 0, 0));

        // When enabled, indicator frame should be reserved space (top placement by default)
        let group_enabled = &groups_enabled[0];
        assert_eq!(group_enabled.indicator_frame, rect(0, 0, 1000, 20)); // Indicator frame at top

        // Window frames should be smaller when indicators are enabled
        // (accounting for the 20px reserved for indicator)
        let target_wid = WindowId::new(1, 1);
        let window_frame_disabled =
            frames_disabled.iter().find(|(wid, _)| *wid == target_wid).unwrap().1;
        let window_frame_enabled =
            frames_enabled.iter().find(|(wid, _)| *wid == target_wid).unwrap().1;

        assert_eq!(window_frame_disabled, rect(0, 0, 1000, 1000));
        assert_eq!(window_frame_enabled, rect(0, 20, 1000, 980));
    }

    #[test]
    fn it_respects_outer_gap() {
        let mut tree = LayoutTree::new();
        let layout = tree.create_layout();
        let root = tree.root(layout);
        let _a1 = tree.add_window_under(layout, root, WindowId::new(1, 1));
        let _a2 = tree.add_window_under(layout, root, WindowId::new(1, 2));

        let screen = rect(0, 0, 1000, 1000);
        let outer_gap = 10.0;

        // Test with non-fullscreen layout
        let mut config = Config::default();
        config.settings.outer_gap = outer_gap;
        let (frames, _) = tree.calculate_layout_and_groups(layout, screen, &config);

        // Without fullscreen, windows should be split with outer_gap applied at root
        // Screen (0,0,1000,1000) with outer_gap=10 becomes (10,10,980,980)
        // Then split horizontally: each window gets half of 980 = 490
        let window1_frame = frames.iter().find(|(wid, _)| *wid == WindowId::new(1, 1)).unwrap().1;
        let window2_frame = frames.iter().find(|(wid, _)| *wid == WindowId::new(1, 2)).unwrap().1;
        assert_eq!(
            window1_frame,
            rect(10, 10, 490, 980),
            "window1 before fullscreen"
        );
        assert_eq!(
            window2_frame,
            rect(500, 10, 490, 980),
            "window2 before fullscreen"
        );

        // Mark first window as fullscreen
        let node1 = tree.window_node(layout, WindowId::new(1, 1)).unwrap();
        tree.set_fullscreen(node1, true);

        let (frames_fullscreen, _) = tree.calculate_layout_and_groups(layout, screen, &config);

        // Fullscreen window should respect outer_gap (inset on all sides from 0,0,1000,1000)
        let window1_fullscreen_frame =
            frames_fullscreen.iter().find(|(wid, _)| *wid == WindowId::new(1, 1)).unwrap().1;

        // Expected: (10, 10, 980, 980) - fullscreen with outer_gap
        assert_eq!(
            window1_fullscreen_frame,
            rect(10, 10, 980, 980),
            "window1 fullscreen with outer_gap"
        );
    }

    #[test]
    fn it_gives_a_constrained_window_its_minimum_size() {
        let mut tree = LayoutTree::new();
        let layout = tree.create_layout();
        let root = tree.root(layout);
        let w1 = WindowId::new(1, 1);
        let w2 = WindowId::new(1, 2);
        let w3 = WindowId::new(1, 3);
        tree.add_window_under(layout, root, w1);
        tree.add_window_under(layout, root, w2);
        tree.add_window_under(layout, root, w3);
        tree.note_window_min_size(w2, CGSize::new(700.0, 0.0));

        let screen = rect(0, 0, 1450, 1000);
        let (mut frames, _) = tree.calculate_layout_and_groups(layout, screen, &Config::default());
        frames.sort_by_key(|&(wid, _)| wid);
        assert_eq!(
            frames,
            vec![
                (w1, rect(0, 0, 375, 1000)),
                (w2, rect(375, 0, 700, 1000)),
                (w3, rect(1075, 0, 375, 1000)),
            ]
        );
    }

    #[test]
    fn it_propagates_minimums_out_of_nested_containers() {
        let mut tree = LayoutTree::new();
        let layout = tree.create_layout();
        let root = tree.root(layout);
        let w1 = WindowId::new(1, 1);
        let w2 = WindowId::new(1, 2);
        let w3 = WindowId::new(1, 3);
        let nested = tree.add_container(root, ContainerKind::Horizontal);
        tree.add_window_under(layout, nested, w1);
        tree.add_window_under(layout, nested, w2);
        tree.add_window_under(layout, root, w3);
        tree.note_window_min_size(w1, CGSize::new(200.0, 0.0));
        tree.note_window_min_size(w2, CGSize::new(300.0, 0.0));

        let screen = rect(0, 0, 600, 1000);
        let (mut frames, _) = tree.calculate_layout_and_groups(layout, screen, &Config::default());
        frames.sort_by_key(|&(wid, _)| wid);
        assert_eq!(
            frames,
            vec![
                (w1, rect(0, 0, 200, 1000)),
                (w2, rect(200, 0, 300, 1000)),
                (w3, rect(500, 0, 100, 1000)),
            ]
        );
    }

    /// When even the minimums don't fit, the frames overlap instead of every
    /// window being squeezed below its minimum. Keeping the frames on screen
    /// is the caller's job.
    #[test]
    fn it_overlaps_constrained_windows_when_space_runs_out() {
        let mut tree = LayoutTree::new();
        let layout = tree.create_layout();
        let root = tree.root(layout);
        let w1 = WindowId::new(1, 1);
        let w2 = WindowId::new(1, 2);
        tree.add_window_under(layout, root, w1);
        tree.add_window_under(layout, root, w2);
        tree.note_window_min_size(w1, CGSize::new(700.0, 0.0));
        tree.note_window_min_size(w2, CGSize::new(700.0, 0.0));

        let screen = rect(0, 0, 1000, 1000);
        let (mut frames, _) = tree.calculate_layout_and_groups(layout, screen, &Config::default());
        frames.sort_by_key(|&(wid, _)| wid);
        assert_eq!(
            frames,
            vec![(w1, rect(0, 0, 700, 1000)), (w2, rect(700, 0, 700, 1000)),]
        );
    }

    /// A window without a constraint keeps its proportional share even when a
    /// neighbor's minimum doesn't fit.
    #[test]
    fn it_keeps_a_share_for_unconstrained_windows_when_space_runs_out() {
        let mut tree = LayoutTree::new();
        let layout = tree.create_layout();
        let root = tree.root(layout);
        let w1 = WindowId::new(1, 1);
        let w2 = WindowId::new(1, 2);
        let w3 = WindowId::new(1, 3);
        tree.add_window_under(layout, root, w1);
        tree.add_window_under(layout, root, w2);
        tree.add_window_under(layout, root, w3);
        tree.note_window_min_size(w1, CGSize::new(700.0, 0.0));
        tree.note_window_min_size(w2, CGSize::new(700.0, 0.0));

        let screen = rect(0, 0, 1000, 1000);
        let (mut frames, _) = tree.calculate_layout_and_groups(layout, screen, &Config::default());
        frames.sort_by_key(|&(wid, _)| wid);
        assert_eq!(
            frames,
            vec![
                (w1, rect(0, 0, 700, 1000)),
                (w2, rect(700, 0, 700, 1000)),
                (w3, rect(1400, 0, 333, 1000)),
            ]
        );
    }

    /// A grouped window's minimum includes the indicator bar's thickness, and
    /// the minimum of a nested container reaches its parent.
    #[test]
    fn it_includes_group_bars_in_a_grouped_windows_minimum() {
        let mut tree = LayoutTree::new();
        let layout = tree.create_layout();
        let root = tree.root(layout);
        let w1 = WindowId::new(1, 1);
        let w2 = WindowId::new(1, 2);
        // A vertical root so the tabbed group's height is constrained by its
        // own minimum rather than the screen.
        tree.set_container_kind(root, ContainerKind::Vertical);
        let group = tree.add_container(root, ContainerKind::Tabbed);
        tree.add_window_under(layout, group, w1);
        tree.add_window_under(layout, group, w2);
        tree.note_window_min_size(w1, CGSize::new(0.0, 300.0));
        tree.note_window_min_size(w2, CGSize::new(0.0, 250.0));

        let mut config = Config::default();
        config.settings.group_bars.enable = true;
        config.settings.group_bars.thickness = 20.0;

        // The group needs 300 for its tallest child plus 20 for the bar, so a
        // 300 point screen can't fit it.
        let screen = rect(0, 0, 1000, 300);
        let (frames, _) = tree.calculate_layout_and_groups(layout, screen, &config);
        let group_frame = frames.iter().find(|(wid, _)| *wid == w1).unwrap().1;
        assert_eq!(group_frame, rect(0, 20, 1000, 300));

        // With room to spare the group keeps its weight-based share.
        let screen = rect(0, 0, 1000, 800);
        let (frames, _) = tree.calculate_layout_and_groups(layout, screen, &config);
        let group_frame = frames.iter().find(|(wid, _)| *wid == w1).unwrap().1;
        assert_eq!(group_frame, rect(0, 20, 1000, 780));
    }
}
