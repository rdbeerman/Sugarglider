// Copyright The Glide Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Contexts: named sets of windows that the user switches between.
//!
//! This module holds membership, the active context, and the pure decisions
//! about which windows to show. It does no I/O. Most-recently-used order is a
//! sequence number that increments on each switch or focus, not a timestamp.
//! The design is in `docs/specs/contexts.md`.

use crate::actor::app::WindowId;
use crate::collections::HashMap;
use crate::sys::window_server::WindowServerId;

/// Identifies a named context. Ids are never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContextId(u32);

impl ContextId {
    pub fn get(self) -> u32 {
        self.0
    }
}

/// A context the user can switch to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ContextKey {
    /// Shows every window in the Space's normal layout.
    Everything,
    /// Shows the windows that belong to no named context.
    Unsorted,
    Named(ContextId),
}

/// How a member record relates to a live window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RecordLink {
    /// No live window. The record can match a window that appears.
    #[default]
    Empty,
    /// The record describes this open window.
    Live(WindowId),
    /// The window closed. The record waits to learn whether its app quit,
    /// and takes part in no matching until then.
    Pending(WindowId),
}

/// The stored description of a member window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberRecord {
    pub bundle_id: Option<String>,
    pub app_name: Option<String>,
    pub title: String,
    pub window_server_id: Option<WindowServerId>,
    pub link: RecordLink,
}

impl MemberRecord {
    fn for_window(window: &WindowDesc) -> Self {
        MemberRecord {
            bundle_id: window.bundle_id.clone(),
            app_name: window.app_name.clone(),
            title: window.title.clone(),
            window_server_id: window.window_server_id,
            link: RecordLink::Live(window.wid),
        }
    }

    /// The open window this record describes.
    pub fn window(&self) -> Option<WindowId> {
        match self.link {
            RecordLink::Live(wid) => Some(wid),
            RecordLink::Empty | RecordLink::Pending(_) => None,
        }
    }
}

/// A live window, as the actor describes it to the model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowDesc {
    pub wid: WindowId,
    pub bundle_id: Option<String>,
    pub app_name: Option<String>,
    pub title: String,
    pub window_server_id: Option<WindowServerId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Context {
    pub id: ContextId,
    pub name: String,
    pub number: Option<u8>,
    pub members: Vec<MemberRecord>,
    /// The value of the use sequence at the last switch to this context.
    pub last_used: u64,
}

impl Context {
    fn has_window(&self, wid: WindowId) -> bool {
        self.members.iter().any(|m| m.window() == Some(wid))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ContextError {
    #[error("A context name can't be empty")]
    EmptyName,
    #[error("\"{0}\" is a reserved name")]
    ReservedName(String),
    #[error("A context named \"{0}\" already exists")]
    NameTaken(String),
    #[error("Context numbers go from 1 to 9, not {0}")]
    NumberOutOfRange(u8),
    #[error("No such context")]
    NoSuchContext,
}

pub const EVERYTHING_NAME: &str = "Everything";
pub const UNSORTED_NAME: &str = "Unsorted";

/// The user's contexts, their members, and the active context.
///
/// Scope is global: one active context covers every screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Contexts {
    contexts: Vec<Context>,
    /// Windows that are members of every context, including later ones.
    pinned: Vec<MemberRecord>,
    next_id: u32,
    use_seq: u64,
    active: ContextKey,
    previous: Option<ContextKey>,
    unsorted_last_used: u64,
    everything_last_used: u64,
    focus_seq: u64,
    last_focus: HashMap<WindowId, u64>,
}

impl Default for Contexts {
    fn default() -> Self {
        Contexts {
            contexts: Vec::new(),
            pinned: Vec::new(),
            next_id: 1,
            use_seq: 0,
            active: ContextKey::Everything,
            previous: None,
            unsorted_last_used: 0,
            everything_last_used: 0,
            focus_seq: 0,
            last_focus: HashMap::default(),
        }
    }
}

impl Contexts {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn contexts(&self) -> &[Context] {
        &self.contexts
    }

    pub fn pinned(&self) -> &[MemberRecord] {
        &self.pinned
    }

    pub fn get(&self, id: ContextId) -> Option<&Context> {
        self.contexts.iter().find(|c| c.id == id)
    }

    fn get_mut(&mut self, id: ContextId) -> Result<&mut Context, ContextError> {
        self.contexts.iter_mut().find(|c| c.id == id).ok_or(ContextError::NoSuchContext)
    }

    pub fn by_number(&self, number: u8) -> Option<&Context> {
        self.contexts.iter().find(|c| c.number == Some(number))
    }

    /// Finds a context by name, ignoring case.
    pub fn by_name(&self, name: &str) -> Option<&Context> {
        let name = name.trim().to_lowercase();
        self.contexts.iter().find(|c| c.name.to_lowercase() == name)
    }

    pub fn active(&self) -> ContextKey {
        self.active
    }

    /// The context that was active before the current one.
    pub fn previous(&self) -> Option<ContextKey> {
        self.previous
    }

    pub fn last_used(&self, key: ContextKey) -> u64 {
        match key {
            ContextKey::Everything => self.everything_last_used,
            ContextKey::Unsorted => self.unsorted_last_used,
            ContextKey::Named(id) => self.get(id).map_or(0, |c| c.last_used),
        }
    }

    fn checked_name(
        &self,
        name: &str,
        renaming: Option<ContextId>,
    ) -> Result<String, ContextError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(ContextError::EmptyName);
        }
        let lower = name.to_lowercase();
        if lower == EVERYTHING_NAME.to_lowercase() || lower == UNSORTED_NAME.to_lowercase() {
            return Err(ContextError::ReservedName(name.to_string()));
        }
        if let Some(other) = self.by_name(name)
            && Some(other.id) != renaming
        {
            return Err(ContextError::NameTaken(other.name.clone()));
        }
        Ok(name.to_string())
    }

    /// Creates an empty context. It gets the lowest free number from 1 to 9,
    /// or none when all are taken.
    pub fn create(&mut self, name: &str) -> Result<ContextId, ContextError> {
        let name = self.checked_name(name, None)?;
        let number = (1..=9).find(|n| self.by_number(*n).is_none());
        let id = ContextId(self.next_id);
        self.next_id += 1;
        self.contexts.push(Context {
            id,
            name,
            number,
            members: Vec::new(),
            last_used: 0,
        });
        Ok(id)
    }

    pub fn rename(&mut self, id: ContextId, name: &str) -> Result<(), ContextError> {
        let name = self.checked_name(name, Some(id))?;
        self.get_mut(id)?.name = name;
        Ok(())
    }

    /// Gives a context a number, or removes its number. A context that
    /// already has the number loses it.
    pub fn set_number(&mut self, id: ContextId, number: Option<u8>) -> Result<(), ContextError> {
        if let Some(n) = number
            && !(1..=9).contains(&n)
        {
            return Err(ContextError::NumberOutOfRange(n));
        }
        self.get_mut(id)?;
        for context in &mut self.contexts {
            if context.id == id {
                context.number = number;
            } else if number.is_some() && context.number == number {
                context.number = None;
            }
        }
        Ok(())
    }

    /// Deletes a context. Its windows stay open; the ones that were only in
    /// this context become unsorted. If it was active, Unsorted becomes
    /// active so that those windows stay visible.
    pub fn delete(&mut self, id: ContextId) -> Result<Context, ContextError> {
        let idx = self
            .contexts
            .iter()
            .position(|c| c.id == id)
            .ok_or(ContextError::NoSuchContext)?;
        let context = self.contexts.remove(idx);
        let key = ContextKey::Named(id);
        if self.active == key {
            self.active = ContextKey::Unsorted;
        }
        if self.previous == Some(key) {
            self.previous = None;
        }
        Ok(context)
    }

    /// Adds a window to a context. Returns false if it was already a member.
    pub fn add_window(&mut self, id: ContextId, window: &WindowDesc) -> Result<bool, ContextError> {
        let context = self.get_mut(id)?;
        if context.has_window(window.wid) {
            return Ok(false);
        }
        context.members.push(MemberRecord::for_window(window));
        Ok(true)
    }

    /// Removes a window from a context. Returns false if it wasn't a member.
    pub fn remove_window(&mut self, id: ContextId, wid: WindowId) -> Result<bool, ContextError> {
        let context = self.get_mut(id)?;
        let len = context.members.len();
        context.members.retain(|m| m.window() != Some(wid));
        Ok(context.members.len() != len)
    }

    /// Moves a window out of the active context and into `target`.
    pub fn move_window(
        &mut self,
        target: ContextId,
        window: &WindowDesc,
    ) -> Result<(), ContextError> {
        self.get_mut(target)?;
        if let ContextKey::Named(active) = self.active
            && active != target
        {
            self.remove_window(active, window.wid)?;
        }
        self.add_window(target, window)?;
        Ok(())
    }

    pub fn is_pinned(&self, wid: WindowId) -> bool {
        self.pinned.iter().any(|m| m.window() == Some(wid))
    }

    /// Makes a window a member of every context. Its records in named
    /// contexts stay, so unpinning leaves it where it was. Returns false if
    /// it was already pinned.
    pub fn pin(&mut self, window: &WindowDesc) -> bool {
        if self.is_pinned(window.wid) {
            return false;
        }
        self.pinned.push(MemberRecord::for_window(window));
        true
    }

    /// Returns false if the window wasn't pinned.
    pub fn unpin(&mut self, wid: WindowId) -> bool {
        let len = self.pinned.len();
        self.pinned.retain(|m| m.window() != Some(wid));
        self.pinned.len() != len
    }

    /// Records a switch to `key`. Switching to the active context again
    /// counts as a use but leaves the previous context alone.
    pub fn switch_to(&mut self, key: ContextKey) -> Result<(), ContextError> {
        if let ContextKey::Named(id) = key {
            self.get_mut(id)?;
        }
        self.use_seq += 1;
        let seq = self.use_seq;
        match key {
            ContextKey::Everything => self.everything_last_used = seq,
            ContextKey::Unsorted => self.unsorted_last_used = seq,
            ContextKey::Named(id) => self.get_mut(id)?.last_used = seq,
        }
        if self.active != key {
            self.previous = Some(self.active);
            self.active = key;
        }
        Ok(())
    }

    /// The named contexts that hold a record of this open window. Pinning is
    /// not included.
    pub fn contexts_of(&self, wid: WindowId) -> Vec<ContextId> {
        self.contexts.iter().filter(|c| c.has_window(wid)).map(|c| c.id).collect()
    }

    /// Whether an open window is in no named context and not pinned.
    pub fn is_unsorted(&self, wid: WindowId) -> bool {
        !self.is_pinned(wid) && !self.contexts.iter().any(|c| c.has_window(wid))
    }

    /// Whether an open window shows when `key` is active.
    pub fn is_member(&self, key: ContextKey, wid: WindowId) -> bool {
        match key {
            ContextKey::Everything => true,
            _ if self.is_pinned(wid) => true,
            ContextKey::Unsorted => self.is_unsorted(wid),
            ContextKey::Named(id) => self.get(id).is_some_and(|c| c.has_window(wid)),
        }
    }

    /// The context to switch to when the user focuses this window from
    /// outside the active context: the most recently used context that holds
    /// it, or Unsorted.
    pub fn focus_target(&self, wid: WindowId) -> ContextKey {
        if self.is_member(self.active, wid) {
            return self.active;
        }
        self.contexts
            .iter()
            .filter(|c| c.has_window(wid))
            .max_by_key(|c| c.last_used)
            .map_or(ContextKey::Unsorted, |c| ContextKey::Named(c.id))
    }

    /// Records that a window took focus.
    pub fn window_focused(&mut self, wid: WindowId) {
        self.focus_seq += 1;
        self.last_focus.insert(wid, self.focus_seq);
    }

    /// When a window last took focus, as a sequence number. Larger is later.
    pub fn last_focus(&self, wid: WindowId) -> Option<u64> {
        self.last_focus.get(&wid).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wid(pid: i32, idx: u32) -> WindowId {
        WindowId::new(pid, idx)
    }

    fn window(pid: i32, idx: u32, app: &str, title: &str) -> WindowDesc {
        WindowDesc {
            wid: wid(pid, idx),
            bundle_id: Some(format!("com.example.{app}")),
            app_name: Some(app.to_string()),
            title: title.to_string(),
            window_server_id: Some(WindowServerId(pid as u32 * 1000 + idx)),
        }
    }

    fn named(id: ContextId) -> ContextKey {
        ContextKey::Named(id)
    }

    #[test]
    fn r1_window_can_be_in_several_contexts() {
        let mut cx = Contexts::new();
        let comms = cx.create("Comms").unwrap();
        let relax = cx.create("Relax").unwrap();
        let whatsapp = window(1, 1, "WhatsApp", "WhatsApp");
        assert!(cx.add_window(comms, &whatsapp).unwrap());
        assert!(cx.add_window(relax, &whatsapp).unwrap());
        assert!(!cx.add_window(relax, &whatsapp).unwrap());
        assert_eq!(cx.contexts_of(whatsapp.wid), vec![comms, relax]);
        assert_eq!(cx.get(relax).unwrap().members.len(), 1);
        assert!(!cx.is_unsorted(whatsapp.wid));
        assert!(cx.is_unsorted(wid(2, 1)));
    }

    #[test]
    fn r3_pinned_window_is_member_of_every_context_including_later_ones() {
        let mut cx = Contexts::new();
        let a = cx.create("A").unwrap();
        let w = window(1, 1, "Music", "Music");
        assert!(cx.pin(&w));
        assert!(!cx.pin(&w));
        let b = cx.create("B").unwrap();
        assert!(cx.is_member(named(a), w.wid));
        assert!(cx.is_member(named(b), w.wid));
        assert!(cx.is_member(ContextKey::Everything, w.wid));
        // A pinned window shows under Unsorted too, but isn't counted as unsorted.
        assert!(cx.is_member(ContextKey::Unsorted, w.wid));
        assert!(!cx.is_unsorted(w.wid));
    }

    #[test]
    fn r3_unpinning_keeps_named_memberships() {
        let mut cx = Contexts::new();
        let a = cx.create("A").unwrap();
        let b = cx.create("B").unwrap();
        let w = window(1, 1, "Music", "Music");
        cx.add_window(a, &w).unwrap();
        cx.pin(&w);
        assert!(cx.unpin(w.wid));
        assert!(!cx.unpin(w.wid));
        assert!(cx.is_member(named(a), w.wid));
        assert!(!cx.is_member(named(b), w.wid));
    }

    #[test]
    fn r4_names_are_unique_ignoring_case() {
        let mut cx = Contexts::new();
        let comms = cx.create("Comms").unwrap();
        assert_eq!(cx.create("comms"), Err(ContextError::NameTaken("Comms".into())));
        assert_eq!(cx.create("  "), Err(ContextError::EmptyName));
        let other = cx.create("Other").unwrap();
        assert_eq!(
            cx.rename(other, "COMMS"),
            Err(ContextError::NameTaken("Comms".into()))
        );
        cx.rename(comms, "COMMS").unwrap();
        assert_eq!(cx.get(comms).unwrap().name, "COMMS");
        assert_eq!(cx.by_name("comms").unwrap().id, comms);
    }

    #[test]
    fn r4_everything_and_unsorted_are_reserved() {
        let mut cx = Contexts::new();
        assert_eq!(
            cx.create("everything"),
            Err(ContextError::ReservedName("everything".into()))
        );
        assert_eq!(
            cx.create("Unsorted "),
            Err(ContextError::ReservedName("Unsorted".into()))
        );
        let a = cx.create("A").unwrap();
        assert_eq!(
            cx.rename(a, "UNSORTED"),
            Err(ContextError::ReservedName("UNSORTED".into()))
        );
    }

    #[test]
    fn r5_new_context_gets_lowest_free_number() {
        let mut cx = Contexts::new();
        let a = cx.create("A").unwrap();
        let b = cx.create("B").unwrap();
        let c = cx.create("C").unwrap();
        assert_eq!(cx.get(b).unwrap().number, Some(2));
        cx.delete(b).unwrap();
        cx.set_number(a, None).unwrap();
        let d = cx.create("D").unwrap();
        let e = cx.create("E").unwrap();
        assert_eq!(cx.get(d).unwrap().number, Some(1));
        assert_eq!(cx.get(e).unwrap().number, Some(2));
        assert_eq!(cx.get(c).unwrap().number, Some(3));
    }

    #[test]
    fn r5_no_number_when_one_to_nine_are_taken() {
        let mut cx = Contexts::new();
        for n in 1..=9 {
            let id = cx.create(&format!("C{n}")).unwrap();
            assert_eq!(cx.get(id).unwrap().number, Some(n));
        }
        let tenth = cx.create("C10").unwrap();
        assert_eq!(cx.get(tenth).unwrap().number, None);
    }

    #[test]
    fn r5_numbers_stay_unique() {
        let mut cx = Contexts::new();
        let a = cx.create("A").unwrap();
        let b = cx.create("B").unwrap();
        cx.set_number(b, Some(1)).unwrap();
        assert_eq!(cx.get(a).unwrap().number, None);
        assert_eq!(cx.get(b).unwrap().number, Some(1));
        assert_eq!(cx.by_number(1).unwrap().id, b);
        assert_eq!(cx.set_number(a, Some(0)), Err(ContextError::NumberOutOfRange(0)));
        assert_eq!(
            cx.set_number(a, Some(10)),
            Err(ContextError::NumberOutOfRange(10))
        );
        assert_eq!(
            cx.set_number(ContextId(99), Some(3)),
            Err(ContextError::NoSuchContext)
        );
        assert_eq!(cx.get(b).unwrap().number, Some(1));
    }

    #[test]
    fn r6_deleting_context_leaves_its_only_members_unsorted() {
        let mut cx = Contexts::new();
        let a = cx.create("A").unwrap();
        let b = cx.create("B").unwrap();
        let only_a = window(1, 1, "App", "One");
        let shared = window(1, 2, "App", "Two");
        cx.add_window(a, &only_a).unwrap();
        cx.add_window(a, &shared).unwrap();
        cx.add_window(b, &shared).unwrap();
        cx.switch_to(named(b)).unwrap();
        let deleted = cx.delete(a).unwrap();
        assert_eq!(deleted.members.len(), 2);
        assert!(cx.is_unsorted(only_a.wid));
        assert_eq!(cx.contexts_of(shared.wid), vec![b]);
        assert_eq!(cx.active(), named(b));
        assert_eq!(cx.delete(a), Err(ContextError::NoSuchContext));
    }

    #[test]
    fn r6_deleting_active_context_activates_unsorted() {
        let mut cx = Contexts::new();
        let a = cx.create("A").unwrap();
        cx.switch_to(named(a)).unwrap();
        cx.delete(a).unwrap();
        assert_eq!(cx.active(), ContextKey::Unsorted);
    }

    #[test]
    fn r6_deleting_previous_context_clears_previous() {
        let mut cx = Contexts::new();
        let a = cx.create("A").unwrap();
        let b = cx.create("B").unwrap();
        cx.switch_to(named(a)).unwrap();
        cx.switch_to(named(b)).unwrap();
        assert_eq!(cx.previous(), Some(named(a)));
        cx.delete(a).unwrap();
        assert_eq!(cx.previous(), None);
    }

    #[test]
    fn r18_previous_context_is_the_one_used_before() {
        let mut cx = Contexts::new();
        let a = cx.create("A").unwrap();
        let b = cx.create("B").unwrap();
        assert_eq!(cx.previous(), None);
        cx.switch_to(named(a)).unwrap();
        assert_eq!(cx.previous(), Some(ContextKey::Everything));
        cx.switch_to(named(b)).unwrap();
        assert_eq!(cx.previous(), Some(named(a)));
        cx.switch_to(ContextKey::Unsorted).unwrap();
        assert_eq!(cx.previous(), Some(named(b)));
    }

    #[test]
    fn r16_switching_to_the_active_context_keeps_previous() {
        let mut cx = Contexts::new();
        let a = cx.create("A").unwrap();
        let b = cx.create("B").unwrap();
        cx.switch_to(named(a)).unwrap();
        cx.switch_to(named(b)).unwrap();
        cx.switch_to(named(b)).unwrap();
        assert_eq!(cx.active(), named(b));
        assert_eq!(cx.previous(), Some(named(a)));
    }

    #[test]
    fn r19_each_switch_updates_most_recently_used_order() {
        let mut cx = Contexts::new();
        let a = cx.create("A").unwrap();
        let b = cx.create("B").unwrap();
        assert_eq!(cx.last_used(named(a)), 0);
        cx.switch_to(named(a)).unwrap();
        cx.switch_to(named(b)).unwrap();
        assert!(cx.last_used(named(b)) > cx.last_used(named(a)));
        cx.switch_to(ContextKey::Unsorted).unwrap();
        cx.switch_to(named(a)).unwrap();
        assert!(cx.last_used(named(a)) > cx.last_used(ContextKey::Unsorted));
        assert!(cx.last_used(ContextKey::Unsorted) > cx.last_used(named(b)));
        assert_eq!(cx.last_used(ContextKey::Everything), 0);
        assert_eq!(
            cx.switch_to(named(ContextId(99))),
            Err(ContextError::NoSuchContext)
        );
    }

    #[test]
    fn r24_focus_target_is_most_recently_used_context_holding_the_window() {
        let mut cx = Contexts::new();
        let comms = cx.create("Comms").unwrap();
        let relax = cx.create("Relax").unwrap();
        let work = cx.create("Work").unwrap();
        let whatsapp = window(1, 1, "WhatsApp", "WhatsApp");
        cx.add_window(comms, &whatsapp).unwrap();
        cx.add_window(relax, &whatsapp).unwrap();
        // Under Everything, focus never switches.
        assert_eq!(cx.focus_target(whatsapp.wid), ContextKey::Everything);
        cx.switch_to(named(relax)).unwrap();
        cx.switch_to(named(comms)).unwrap();
        cx.switch_to(named(work)).unwrap();
        assert_eq!(cx.focus_target(whatsapp.wid), named(comms));
        assert_eq!(cx.focus_target(wid(2, 1)), ContextKey::Unsorted);
        cx.switch_to(named(relax)).unwrap();
        assert_eq!(cx.focus_target(whatsapp.wid), named(relax));
    }

    #[test]
    fn focus_order_is_a_sequence() {
        let mut cx = Contexts::new();
        assert_eq!(cx.last_focus(wid(1, 1)), None);
        cx.window_focused(wid(1, 1));
        cx.window_focused(wid(1, 2));
        assert!(cx.last_focus(wid(1, 2)) > cx.last_focus(wid(1, 1)));
    }

    #[test]
    fn r28_everything_is_active_until_a_switch() {
        let mut cx = Contexts::new();
        assert_eq!(cx.active(), ContextKey::Everything);
        cx.create("A").unwrap();
        assert_eq!(cx.active(), ContextKey::Everything);
    }

    #[test]
    fn move_window_leaves_the_active_context() {
        let mut cx = Contexts::new();
        let a = cx.create("A").unwrap();
        let b = cx.create("B").unwrap();
        let c = cx.create("C").unwrap();
        let w = window(1, 1, "App", "W");
        cx.add_window(a, &w).unwrap();
        cx.add_window(c, &w).unwrap();
        cx.switch_to(named(a)).unwrap();
        cx.move_window(b, &w).unwrap();
        assert_eq!(cx.contexts_of(w.wid), vec![b, c]);
        assert_eq!(
            cx.move_window(ContextId(99), &w),
            Err(ContextError::NoSuchContext)
        );
    }

    #[test]
    fn move_window_under_everything_only_adds() {
        let mut cx = Contexts::new();
        let a = cx.create("A").unwrap();
        let b = cx.create("B").unwrap();
        let w = window(1, 1, "App", "W");
        cx.add_window(a, &w).unwrap();
        cx.move_window(b, &w).unwrap();
        assert_eq!(cx.contexts_of(w.wid), vec![a, b]);
    }

    #[test]
    fn remove_window_removes_one_membership() {
        let mut cx = Contexts::new();
        let a = cx.create("A").unwrap();
        let b = cx.create("B").unwrap();
        let w = window(1, 1, "App", "W");
        cx.add_window(a, &w).unwrap();
        cx.add_window(b, &w).unwrap();
        assert!(cx.remove_window(a, w.wid).unwrap());
        assert!(!cx.remove_window(a, w.wid).unwrap());
        assert_eq!(cx.contexts_of(w.wid), vec![b]);
    }
}
