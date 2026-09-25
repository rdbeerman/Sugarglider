// Copyright The Glide Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Contexts: named sets of windows that the user switches between.
//!
//! This module holds membership, the active context, and the pure decisions
//! about which windows to show. It does no I/O. Most-recently-used order is a
//! sequence number that increments on each switch or focus, not a timestamp.
//! The design is in `docs/specs/contexts.md`.

use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};

use crate::actor::app::{WindowId, pid_t};
use crate::collections::HashMap;
use crate::sys::window_server::WindowServerId;

/// Identifies a named context. Ids are never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberRecord {
    #[serde(default)]
    pub bundle_id: Option<String>,
    #[serde(default)]
    pub app_name: Option<String>,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub window_server_id: Option<WindowServerId>,
    #[serde(skip)]
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    pub id: ContextId,
    pub name: String,
    #[serde(default)]
    pub number: Option<u8>,
    #[serde(default)]
    pub members: Vec<MemberRecord>,
    /// The value of the use sequence at the last switch to this context.
    #[serde(default)]
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
///
/// Serializes to the shape of `contexts.json`. Live windows, pending
/// states, the previous context, and focus order are not saved.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "ContextsFile", try_from = "ContextsFile")]
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

/// Lowercases text and removes accents, for comparing names and titles.
///
/// Folds the Latin-1 Supplement and Latin Extended-A blocks to ASCII and
/// drops combining diacritical marks.
pub fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if ('\u{0300}'..='\u{036F}').contains(&c) {
            continue;
        }
        match fold_char(c) {
            Some(folded) => out.push_str(folded),
            None => out.extend(c.to_lowercase()),
        }
    }
    out
}

fn fold_char(c: char) -> Option<&'static str> {
    Some(match c {
        'À'..='Å' | 'à'..='å' | '\u{0100}'..='\u{0105}' => "a",
        'Æ' | 'æ' => "ae",
        'Ç' | 'ç' | '\u{0106}'..='\u{010D}' => "c",
        'Ð' | 'ð' | '\u{010E}'..='\u{0111}' => "d",
        'È'..='Ë' | 'è'..='ë' | '\u{0112}'..='\u{011B}' => "e",
        '\u{011C}'..='\u{0123}' => "g",
        '\u{0124}'..='\u{0127}' => "h",
        'Ì'..='Ï' | 'ì'..='ï' | '\u{0128}'..='\u{0131}' => "i",
        '\u{0132}' | '\u{0133}' => "ij",
        '\u{0134}' | '\u{0135}' => "j",
        '\u{0136}'..='\u{0138}' => "k",
        '\u{0139}'..='\u{0142}' => "l",
        'Ñ' | 'ñ' | '\u{0143}'..='\u{014B}' => "n",
        'Ò'..='Ö' | 'Ø' | 'ò'..='ö' | 'ø' | '\u{014C}'..='\u{0151}' => "o",
        '\u{0152}' | '\u{0153}' => "oe",
        '\u{0154}'..='\u{0159}' => "r",
        'ß' => "ss",
        '\u{015A}'..='\u{0161}' | '\u{017F}' => "s",
        '\u{0162}'..='\u{0167}' => "t",
        'Þ' | 'þ' => "th",
        'Ù'..='Ü' | 'ù'..='ü' | '\u{0168}'..='\u{0173}' => "u",
        '\u{0174}' | '\u{0175}' => "w",
        'Ý' | 'ý' | 'ÿ' | '\u{0176}'..='\u{0178}' => "y",
        '\u{0179}'..='\u{017E}' => "z",
        _ => return None,
    })
}

/// How well a query matches a context name, from weakest to strongest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum NameMatch {
    /// The query is empty, so every entry is listed.
    EmptyQuery,
    /// The query's letters appear in the name in order.
    LettersInOrder,
    /// Each word of the query starts a word of the name, in order.
    AllWordPrefixes,
    /// The query starts the name's initials.
    Initials,
    /// The query starts a word of the name.
    WordPrefix,
    /// The query starts the name.
    NamePrefix,
    Exact,
}

/// Ranks the switcher's entries for a query, best first.
///
/// The entries are the named contexts, Unsorted when it has windows, and
/// Everything. Entries that don't match are left out. Ties go to the most
/// recently used entry.
pub fn rank(
    query: &str,
    contexts: &Contexts,
    unsorted_has_windows: bool,
) -> Vec<(ContextKey, NameMatch)> {
    let query = fold(query.trim());
    let mut entries: Vec<(ContextKey, &str)> = contexts
        .contexts
        .iter()
        .map(|c| (ContextKey::Named(c.id), c.name.as_str()))
        .collect();
    if unsorted_has_windows {
        entries.push((ContextKey::Unsorted, UNSORTED_NAME));
    }
    entries.push((ContextKey::Everything, EVERYTHING_NAME));
    let mut ranked: Vec<(ContextKey, NameMatch)> = entries
        .into_iter()
        .filter_map(|(key, name)| match_name(&query, &fold(name)).map(|m| (key, m)))
        .collect();
    ranked.sort_by(|(a_key, a), (b_key, b)| {
        b.cmp(a)
            .then_with(|| contexts.last_used(*b_key).cmp(&contexts.last_used(*a_key)))
    });
    ranked
}

/// Matches a folded query against a folded name.
fn match_name(query: &str, name: &str) -> Option<NameMatch> {
    if query.is_empty() {
        return Some(NameMatch::EmptyQuery);
    }
    if name == query {
        return Some(NameMatch::Exact);
    }
    if name.starts_with(query) {
        return Some(NameMatch::NamePrefix);
    }
    if word_starts(name).any(|start| name[start..].starts_with(query)) {
        return Some(NameMatch::WordPrefix);
    }
    let name_words: Vec<&str> = words(name).collect();
    let initials: String = name_words.iter().filter_map(|w| w.chars().next()).collect();
    if !query.contains(char::is_whitespace) && initials.starts_with(query) {
        return Some(NameMatch::Initials);
    }
    let mut remaining = name_words.iter();
    if words(query).all(|q| remaining.any(|w| w.starts_with(q))) {
        return Some(NameMatch::AllWordPrefixes);
    }
    let mut name_chars = name.chars();
    if query.chars().filter(|c| !c.is_whitespace()).all(|q| name_chars.any(|c| c == q)) {
        return Some(NameMatch::LettersInOrder);
    }
    None
}

fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty())
}

/// Byte offsets at which a word starts.
fn word_starts(text: &str) -> impl Iterator<Item = usize> {
    let mut previous_alphanumeric = false;
    text.char_indices().filter_map(move |(i, c)| {
        let starts = c.is_alphanumeric() && !previous_alphanumeric;
        previous_alphanumeric = c.is_alphanumeric();
        starts.then_some(i)
    })
}

/// When window matching runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchPass {
    /// A window appeared. Steps 1 to 3 run.
    Arrival,
    /// A switch is in progress. Steps 1 to 4 run.
    Switch,
}

/// The step at which a window matched a member record, in the order the
/// steps run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchStep {
    /// Same window server id.
    WindowServerId,
    /// Same app and exactly the same title.
    ExactTitle,
    /// Same app and a similar title.
    SimilarTitle,
    /// Same app.
    SameApp,
}

/// Where a member record lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    Context(ContextId),
    Pinned,
}

/// A member record that a window matched. `index` points into the slot's
/// records and is valid until the next change to the contexts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordMatch {
    pub slot: Slot,
    pub index: usize,
    pub step: MatchStep,
}

/// Finds the empty member records that a window matches.
///
/// The steps run in order. Each context, and the pinned list, gives at most
/// one record, from the earliest step that matches in it. Steps 3 and 4 run
/// only when steps 1 and 2 match nothing and the window belongs to no
/// context, so they never take a window that belongs to another context.
/// Step 4 runs only during a switch. Pending records match nothing.
pub fn match_window(window: &WindowDesc, contexts: &Contexts, pass: MatchPass) -> Vec<RecordMatch> {
    let slots = contexts
        .contexts
        .iter()
        .map(|c| (Slot::Context(c.id), &c.members))
        .chain(std::iter::once((Slot::Pinned, &contexts.pinned)));
    let candidates: Vec<(Slot, &Vec<MemberRecord>)> = slots
        .filter(|(_, records)| !records.iter().any(|m| m.window() == Some(window.wid)))
        .collect();
    let is_member = contexts.is_pinned(window.wid) || !contexts.contexts_of(window.wid).is_empty();

    let mut matches = Vec::new();
    for step in [MatchStep::WindowServerId, MatchStep::ExactTitle] {
        match_step(window, &candidates, step, &mut matches);
    }
    if !matches.is_empty() || is_member {
        return matches;
    }
    match_step(window, &candidates, MatchStep::SimilarTitle, &mut matches);
    if matches.is_empty() && pass == MatchPass::Switch {
        match_step(window, &candidates, MatchStep::SameApp, &mut matches);
    }
    matches
}

fn match_step(
    window: &WindowDesc,
    candidates: &[(Slot, &Vec<MemberRecord>)],
    step: MatchStep,
    matches: &mut Vec<RecordMatch>,
) {
    for (slot, records) in candidates {
        if matches.iter().any(|m| m.slot == *slot) {
            continue;
        }
        let found = records.iter().position(|record| {
            record.link == RecordLink::Empty
                && match step {
                    MatchStep::WindowServerId => {
                        window.window_server_id.is_some()
                            && record.window_server_id == window.window_server_id
                    }
                    MatchStep::ExactTitle => {
                        same_app(record, window) && record.title == window.title
                    }
                    MatchStep::SimilarTitle => {
                        same_app(record, window) && similar_titles(&record.title, &window.title)
                    }
                    MatchStep::SameApp => same_app(record, window),
                }
        });
        if let Some(index) = found {
            matches.push(RecordMatch { slot: *slot, index, step });
        }
    }
}

/// Compares bundle ids when both are known, and app names otherwise.
fn same_app(record: &MemberRecord, window: &WindowDesc) -> bool {
    match (&record.bundle_id, &window.bundle_id) {
        (Some(a), Some(b)) => a == b,
        _ => matches!((&record.app_name, &window.app_name), (Some(a), Some(b)) if a == b),
    }
}

/// Whether two window titles are similar: after folding, both have at least
/// 4 characters, and one contains the other or they share a prefix of at
/// least min(12, two-thirds of the shorter title).
pub fn similar_titles(a: &str, b: &str) -> bool {
    let a = fold(a);
    let b = fold(b);
    let a_len = a.chars().count();
    let b_len = b.chars().count();
    if a_len < 4 || b_len < 4 {
        return false;
    }
    if a.contains(&b) || b.contains(&a) {
        return true;
    }
    let shorter = a_len.min(b_len);
    let prefix = a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count();
    prefix >= 12 || prefix * 3 >= shorter * 2
}

/// What happened to a window that appeared.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Arrival {
    /// It matched member records and rejoined the contexts that hold them.
    Rejoined(Vec<RecordMatch>),
    /// It matched nothing and joined the active context of its screen.
    Joined(ContextId),
    /// It matched nothing, and its screen shows Everything or Unsorted.
    Unsorted,
    /// It is already a member of a context or pinned, so nothing changed.
    AlreadyMember,
}

impl Contexts {
    fn slot_records_mut(&mut self, slot: Slot) -> &mut Vec<MemberRecord> {
        match slot {
            Slot::Pinned => &mut self.pinned,
            Slot::Context(id) => {
                &mut self
                    .contexts
                    .iter_mut()
                    .find(|c| c.id == id)
                    .expect("matched context exists")
                    .members
            }
        }
    }

    fn records_mut(&mut self) -> impl Iterator<Item = &mut MemberRecord> {
        self.contexts
            .iter_mut()
            .flat_map(|c| c.members.iter_mut())
            .chain(self.pinned.iter_mut())
    }

    /// Matches a window against the member records and binds it to the
    /// records it matches. The records take the window's current details.
    pub fn rejoin(&mut self, window: &WindowDesc, pass: MatchPass) -> Vec<RecordMatch> {
        let matches = match_window(window, self, pass);
        for m in &matches {
            self.slot_records_mut(m.slot)[m.index] = MemberRecord::for_window(window);
        }
        matches
    }

    /// Decides the membership of a window that just appeared on a screen
    /// that shows `screen_active`.
    ///
    /// A window that matches member records rejoins their contexts and does
    /// not join the active context. Otherwise it joins the active context,
    /// or stays unsorted under Everything and Unsorted. Call this once per
    /// window, when it first appears; a repeated call can't tell an unsorted
    /// window from a new one.
    pub fn window_appeared(&mut self, window: &WindowDesc, screen_active: ContextKey) -> Arrival {
        if !self.is_unsorted(window.wid) {
            return Arrival::AlreadyMember;
        }
        let matches = self.rejoin(window, MatchPass::Arrival);
        if !matches.is_empty() {
            return Arrival::Rejoined(matches);
        }
        if let ContextKey::Named(id) = screen_active
            && let Ok(context) = self.get_mut(id)
        {
            context.members.push(MemberRecord::for_window(window));
            return Arrival::Joined(id);
        }
        Arrival::Unsorted
    }

    /// Updates the records of an open window with its new title.
    pub fn title_changed(&mut self, wid: WindowId, title: &str) {
        for record in self.records_mut() {
            if record.window() == Some(wid) {
                record.title = title.to_string();
            }
        }
    }

    /// Marks the records of a closed window as pending until its app shows
    /// whether it quit.
    pub fn window_closed(&mut self, wid: WindowId) {
        self.last_focus.remove(&wid);
        for record in self.records_mut() {
            if record.link == RecordLink::Live(wid) {
                record.link = RecordLink::Pending(wid);
            }
        }
    }

    /// The app quit. All of its records stay, open or pending, and wait for
    /// its windows to appear again.
    pub fn app_terminated(&mut self, pid: pid_t) {
        self.last_focus.retain(|wid, _| wid.pid != pid);
        for record in self.records_mut() {
            if let RecordLink::Live(wid) | RecordLink::Pending(wid) = record.link
                && wid.pid == pid
            {
                record.link = RecordLink::Empty;
            }
        }
    }

    /// The app is still running, so its closed windows were closed for good.
    /// Deletes their pending records.
    pub fn app_still_running(&mut self, pid: pid_t) {
        let still_running =
            |m: &MemberRecord| !matches!(m.link, RecordLink::Pending(wid) if wid.pid == pid);
        for context in &mut self.contexts {
            context.members.retain(still_running);
        }
        self.pinned.retain(still_running);
    }
}

/// The windows of a visible screen, for planning a switch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwitchScreen {
    /// The screen's active context after the switch.
    pub active: ContextKey,
    /// The windows on the screen's Space, with the windows the user can't
    /// see listed with `unseen_space` set or left out.
    pub windows: Vec<SwitchWindow>,
}

/// A window on a visible screen, for planning a switch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwitchWindow {
    pub wid: WindowId,
    /// The named contexts that hold the window.
    pub contexts: Vec<ContextId>,
    pub pinned: bool,
    pub minimized: bool,
    /// The layout manager doesn't track the window.
    pub untracked: bool,
    /// The user hid the window's app.
    pub app_hidden: bool,
    /// The window is only on Spaces nobody can see.
    pub unseen_space: bool,
    pub parked: bool,
    /// The window belongs to Sugarglider.
    pub own: bool,
    /// When the window last took focus, as a sequence number.
    pub last_focus: Option<u64>,
}

impl SwitchWindow {
    fn shows_under(&self, key: ContextKey) -> bool {
        match key {
            ContextKey::Everything => true,
            _ if self.pinned => true,
            ContextKey::Unsorted => self.contexts.is_empty(),
            ContextKey::Named(id) => self.contexts.contains(&id),
        }
    }

    /// Whether the user can see and use the window, and Sugarglider may move it.
    fn in_play(&self) -> bool {
        !(self.own || self.untracked || self.minimized || self.app_hidden || self.unseen_space)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwitchInput {
    pub screens: Vec<SwitchScreen>,
}

/// What a switch does to windows. Windows keep the screen they're on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SwitchPlan {
    /// Windows to park, which must be written to the journal first.
    pub park: Vec<WindowId>,
    /// Parked windows to put back.
    pub unpark: Vec<WindowId>,
    /// The window to focus with a quiet raise.
    pub focus: Option<WindowId>,
}

/// Plans a switch: which windows to park, which to put back, and which to
/// focus.
///
/// A window must show when it is a member of its screen's active context;
/// under Everything every window must show. Windows that must show and are
/// parked are put back. The others are parked, except Sugarglider's own
/// windows, untracked and minimized windows, windows of hidden apps, and
/// windows on Spaces nobody can see. Windows that are parked already stay
/// parked. The focus goes to the most recently focused window that shows
/// and that the user can see.
pub fn plan_switch(input: &SwitchInput) -> SwitchPlan {
    let mut plan = SwitchPlan::default();
    let mut focus: Option<&SwitchWindow> = None;
    for screen in &input.screens {
        for window in &screen.windows {
            if window.shows_under(screen.active) {
                if window.parked {
                    plan.unpark.push(window.wid);
                }
                if window.in_play() && focus.is_none_or(|f| window.last_focus > f.last_focus) {
                    focus = Some(window);
                }
            } else if !window.parked && window.in_play() {
                plan.park.push(window.wid);
            }
        }
    }
    plan.focus = focus.map(|w| w.wid);
    plan
}

impl Contexts {
    /// Describes a window's membership for [`plan_switch`]. The other fields
    /// are left false for the caller to fill in.
    pub fn switch_window(&self, wid: WindowId) -> SwitchWindow {
        SwitchWindow {
            wid,
            contexts: self.contexts_of(wid),
            pinned: self.is_pinned(wid),
            minimized: false,
            untracked: false,
            app_hidden: false,
            unseen_space: false,
            parked: false,
            own: false,
            last_focus: self.last_focus(wid),
        }
    }
}

/// The version of `contexts.json` that this code reads and writes.
pub const CONTEXTS_FILE_VERSION: u32 = 1;

/// The shape of `contexts.json`.
#[derive(Serialize, Deserialize)]
struct ContextsFile {
    version: u32,
    next_id: u32,
    use_seq: u64,
    #[serde(default)]
    contexts: Vec<Context>,
    #[serde(default)]
    pinned: Vec<MemberRecord>,
    #[serde(default)]
    active: Option<SavedActive>,
}

/// `{ "global": <key> }`. Anything else loads as Everything.
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum SavedActive {
    Global {
        global: SavedKey,
    },
    #[serde(skip_serializing)]
    Unknown(IgnoredAny),
}

/// A context id, or `"everything"` or `"unsorted"`.
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum SavedKey {
    Named(ContextId),
    Builtin(BuiltinKey),
    #[serde(skip_serializing)]
    Unknown(IgnoredAny),
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum BuiltinKey {
    Everything,
    Unsorted,
}

impl From<Contexts> for ContextsFile {
    fn from(contexts: Contexts) -> Self {
        let global = match contexts.active {
            ContextKey::Everything => SavedKey::Builtin(BuiltinKey::Everything),
            ContextKey::Unsorted => SavedKey::Builtin(BuiltinKey::Unsorted),
            ContextKey::Named(id) => SavedKey::Named(id),
        };
        ContextsFile {
            version: CONTEXTS_FILE_VERSION,
            next_id: contexts.next_id,
            use_seq: contexts.use_seq,
            contexts: contexts.contexts,
            pinned: contexts.pinned,
            active: Some(SavedActive::Global { global }),
        }
    }
}

impl TryFrom<ContextsFile> for Contexts {
    type Error = String;

    fn try_from(file: ContextsFile) -> Result<Self, String> {
        if file.version != CONTEXTS_FILE_VERSION {
            return Err(format!("unsupported contexts.json version {}", file.version));
        }
        let mut contexts = Contexts {
            next_id: file.next_id,
            use_seq: file.use_seq,
            pinned: file.pinned,
            ..Contexts::default()
        };
        for mut context in file.contexts {
            if contexts.get(context.id).is_some() {
                continue;
            }
            if context
                .number
                .is_some_and(|n| !(1..=9).contains(&n) || contexts.by_number(n).is_some())
            {
                context.number = None;
            }
            contexts.next_id = contexts.next_id.max(context.id.0 + 1);
            contexts.use_seq = contexts.use_seq.max(context.last_used);
            contexts.contexts.push(context);
        }
        contexts.active = match file.active {
            Some(SavedActive::Global {
                global: SavedKey::Builtin(BuiltinKey::Unsorted),
            }) => ContextKey::Unsorted,
            Some(SavedActive::Global { global: SavedKey::Named(id) })
                if contexts.get(id).is_some() =>
            {
                ContextKey::Named(id)
            }
            _ => ContextKey::Everything,
        };
        Ok(contexts)
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

    fn ranked_names(query: &str, cx: &Contexts, unsorted: bool) -> Vec<String> {
        rank(query, cx, unsorted)
            .into_iter()
            .map(|(key, _)| match key {
                ContextKey::Everything => EVERYTHING_NAME.to_string(),
                ContextKey::Unsorted => UNSORTED_NAME.to_string(),
                ContextKey::Named(id) => cx.get(id).unwrap().name.clone(),
            })
            .collect()
    }

    fn match_of(query: &str, name: &str) -> Option<NameMatch> {
        match_name(&fold(query), &fold(name))
    }

    #[test]
    fn rank_prefix_finds_client_work() {
        let mut cx = Contexts::new();
        cx.create("Comms").unwrap();
        cx.create("Client work").unwrap();
        assert_eq!(ranked_names("cli", &cx, false), vec!["Client work"]);
        assert_eq!(match_of("cli", "Client work"), Some(NameMatch::NamePrefix));
    }

    #[test]
    fn rank_initials_find_client_work() {
        let mut cx = Contexts::new();
        cx.create("Comms").unwrap();
        cx.create("Client work").unwrap();
        assert_eq!(ranked_names("cw", &cx, false), vec!["Client work"]);
        assert_eq!(match_of("cw", "Client work"), Some(NameMatch::Initials));
    }

    #[test]
    fn rank_exact_name_beats_prefix() {
        let mut cx = Contexts::new();
        let longer = cx.create("Client work 2").unwrap();
        cx.create("Client work").unwrap();
        cx.switch_to(ContextKey::Named(longer)).unwrap();
        assert_eq!(
            ranked_names("client work", &cx, false),
            vec!["Client work", "Client work 2"]
        );
    }

    #[test]
    fn rank_ignores_accents_and_case() {
        let mut cx = Contexts::new();
        cx.create("Café").unwrap();
        cx.create("Straße").unwrap();
        assert_eq!(ranked_names("cafe", &cx, false), vec!["Café"]);
        assert_eq!(ranked_names("CAFÉ", &cx, false), vec!["Café"]);
        assert_eq!(ranked_names("strasse", &cx, false), vec!["Straße"]);
        // Decomposed accents, as in some file names.
        assert_eq!(ranked_names("Cafe\u{301}", &cx, false), vec!["Café"]);
    }

    #[test]
    fn rank_ties_go_to_most_recently_used() {
        let mut cx = Contexts::new();
        let client = cx.create("Client").unwrap();
        let comms = cx.create("Comms").unwrap();
        cx.switch_to(ContextKey::Named(client)).unwrap();
        cx.switch_to(ContextKey::Named(comms)).unwrap();
        assert_eq!(ranked_names("c", &cx, false), vec!["Comms", "Client"]);
        cx.switch_to(ContextKey::Named(client)).unwrap();
        assert_eq!(ranked_names("c", &cx, false), vec!["Client", "Comms"]);
    }

    #[test]
    fn rank_orders_match_kinds() {
        assert_eq!(match_of("client work", "Client work"), Some(NameMatch::Exact));
        assert_eq!(match_of("client", "Client work"), Some(NameMatch::NamePrefix));
        assert_eq!(match_of("wor", "Client work"), Some(NameMatch::WordPrefix));
        assert_eq!(match_of("cwp", "Client work party"), Some(NameMatch::Initials));
        assert_eq!(
            match_of("cl wo", "Client work"),
            Some(NameMatch::AllWordPrefixes)
        );
        assert_eq!(match_of("clwk", "Client work"), Some(NameMatch::LettersInOrder));
        assert_eq!(match_of("wc", "Client work"), None);
        assert_eq!(match_of("wo cl", "Client work"), None);
        let mut kinds = vec![
            NameMatch::LettersInOrder,
            NameMatch::Exact,
            NameMatch::Initials,
            NameMatch::NamePrefix,
            NameMatch::AllWordPrefixes,
            NameMatch::WordPrefix,
        ];
        kinds.sort();
        kinds.reverse();
        assert_eq!(
            kinds,
            vec![
                NameMatch::Exact,
                NameMatch::NamePrefix,
                NameMatch::WordPrefix,
                NameMatch::Initials,
                NameMatch::AllWordPrefixes,
                NameMatch::LettersInOrder,
            ]
        );
    }

    #[test]
    fn r29_rank_lists_unsorted_only_when_it_has_windows() {
        let mut cx = Contexts::new();
        cx.create("Comms").unwrap();
        assert_eq!(ranked_names("", &cx, false), vec!["Comms", "Everything"]);
        assert_eq!(
            ranked_names("", &cx, true),
            vec!["Comms", "Unsorted", "Everything"]
        );
        assert_eq!(ranked_names("uns", &cx, true), vec!["Unsorted"]);
        assert_eq!(ranked_names("uns", &cx, false), Vec::<String>::new());
        assert_eq!(ranked_names("ev", &cx, true), vec!["Everything"]);
    }

    #[test]
    fn fold_covers_latin_1_and_extended_a() {
        assert_eq!(fold("ÀÉÎÕÜ àéîõü Çç Ññ Ýÿ"), "aeiou aeiou cc nn yy");
        assert_eq!(fold("Æ Œ ß Þ Ð Ø Ĳ"), "ae oe ss th d o ij");
        assert_eq!(fold("Łódź Škoda İstanbul ıi Ħ ſ"), "lodz skoda istanbul ii h s");
        assert_eq!(fold("×÷ 日本"), "×÷ 日本");
    }

    fn empty_record(app: &str, title: &str, wsid: Option<u32>) -> MemberRecord {
        MemberRecord {
            bundle_id: Some(format!("com.example.{app}")),
            app_name: Some(app.to_string()),
            title: title.to_string(),
            window_server_id: wsid.map(WindowServerId),
            link: RecordLink::Empty,
        }
    }

    fn with_records(names_and_records: &[(&str, Vec<MemberRecord>)]) -> (Contexts, Vec<ContextId>) {
        let mut cx = Contexts::new();
        let ids = names_and_records
            .iter()
            .map(|(name, records)| {
                let id = cx.create(name).unwrap();
                cx.get_mut(id).unwrap().members = records.clone();
                id
            })
            .collect();
        (cx, ids)
    }

    fn steps(matches: &[RecordMatch]) -> Vec<(Slot, MatchStep)> {
        matches.iter().map(|m| (m.slot, m.step)).collect()
    }

    #[test]
    fn r22_similar_titles_follow_the_rooms_definition() {
        assert!(similar_titles("Inbox – Mail", "inbox"));
        assert!(similar_titles("Café Notes", "cafe notes (edited)"));
        assert!(!similar_titles("abc", "abc"), "shorter than 4 characters");
        assert!(!similar_titles("Mail", "abc"));
        // Shorter title has 9 characters; two-thirds is 6.
        assert!(similar_titles("Project A", "Projec-99999"));
        assert!(!similar_titles("Project A", "Proje-999999"));
        // A shared prefix of 12 is always enough.
        assert!(similar_titles(
            "Quarterly report draft 1",
            "Quarterly re-something else entirely"
        ));
        assert!(!similar_titles(
            "Quarterly report draft 1",
            "Quarterly r-something else entirely"
        ));
    }

    #[test]
    fn r22_step_1_matches_window_server_id() {
        let (cx, ids) =
            with_records(&[("A", vec![empty_record("Other", "Unrelated", Some(1001))])]);
        let w = window(1, 1, "App", "Title");
        assert_eq!(
            steps(&match_window(&w, &cx, MatchPass::Arrival)),
            vec![(Slot::Context(ids[0]), MatchStep::WindowServerId)]
        );
    }

    #[test]
    fn r22_step_2_matches_same_app_and_exact_title() {
        let (cx, ids) = with_records(&[
            ("A", vec![empty_record("App", "Title", None)]),
            ("B", vec![empty_record("Other", "Title", None)]),
        ]);
        let w = window(1, 1, "App", "Title");
        assert_eq!(
            steps(&match_window(&w, &cx, MatchPass::Arrival)),
            vec![(Slot::Context(ids[0]), MatchStep::ExactTitle)]
        );
    }

    #[test]
    fn r22_step_3_matches_same_app_and_similar_title() {
        let (cx, ids) = with_records(&[("A", vec![empty_record("App", "Inbox – Mail", None)])]);
        let w = window(1, 1, "App", "Inbox");
        assert_eq!(
            steps(&match_window(&w, &cx, MatchPass::Arrival)),
            vec![(Slot::Context(ids[0]), MatchStep::SimilarTitle)]
        );
        let unrelated = window(1, 2, "App", "Drafts");
        assert!(match_window(&unrelated, &cx, MatchPass::Arrival).is_empty());
    }

    #[test]
    fn r22_step_4_only_runs_during_a_switch() {
        let (cx, ids) = with_records(&[("A", vec![empty_record("App", "Something", None)])]);
        let w = window(1, 1, "App", "Different");
        assert!(match_window(&w, &cx, MatchPass::Arrival).is_empty());
        assert_eq!(
            steps(&match_window(&w, &cx, MatchPass::Switch)),
            vec![(Slot::Context(ids[0]), MatchStep::SameApp)]
        );
    }

    #[test]
    fn r22_earlier_steps_win() {
        let (cx, ids) = with_records(&[
            (
                "A",
                vec![
                    empty_record("App", "Inbox – Mail", None),
                    empty_record("App", "Inbox", None),
                ],
            ),
            ("B", vec![empty_record("App", "Inbox – Mail", None)]),
        ]);
        let w = window(1, 1, "App", "Inbox");
        let matches = match_window(&w, &cx, MatchPass::Switch);
        // A's exact record wins; B's similar record isn't taken because the
        // window now belongs to A.
        assert_eq!(
            steps(&matches),
            vec![(Slot::Context(ids[0]), MatchStep::ExactTitle)]
        );
        assert_eq!(matches[0].index, 1);
    }

    #[test]
    fn r22_rejoins_every_context_that_holds_a_matching_record() {
        let (mut cx, ids) = with_records(&[
            ("Comms", vec![empty_record("WhatsApp", "WhatsApp", None)]),
            ("Relax", vec![empty_record("WhatsApp", "WhatsApp", None)]),
            ("Work", vec![]),
        ]);
        let w = window(1, 1, "WhatsApp", "WhatsApp");
        assert_eq!(
            cx.window_appeared(&w, named(ids[2])),
            Arrival::Rejoined(vec![
                RecordMatch {
                    slot: Slot::Context(ids[0]),
                    index: 0,
                    step: MatchStep::ExactTitle
                },
                RecordMatch {
                    slot: Slot::Context(ids[1]),
                    index: 0,
                    step: MatchStep::ExactTitle
                },
            ])
        );
        assert_eq!(cx.contexts_of(w.wid), vec![ids[0], ids[1]]);
        assert_eq!(
            cx.get(ids[0]).unwrap().members[0].window_server_id,
            w.window_server_id
        );
    }

    #[test]
    fn r22_steps_3_and_4_never_take_a_window_of_another_context() {
        let (mut cx, ids) = with_records(&[
            ("A", vec![]),
            (
                "B",
                vec![
                    empty_record("App", "Inbox – Mail", None),
                    empty_record("App", "Other", None),
                ],
            ),
        ]);
        let w = window(1, 1, "App", "Inbox");
        cx.add_window(ids[0], &w).unwrap();
        assert!(match_window(&w, &cx, MatchPass::Arrival).is_empty());
        assert!(match_window(&w, &cx, MatchPass::Switch).is_empty());
        // A pinned window belongs to every context.
        cx.remove_window(ids[0], w.wid).unwrap();
        cx.pin(&w);
        assert!(match_window(&w, &cx, MatchPass::Switch).is_empty());
        cx.unpin(w.wid);
        assert_eq!(match_window(&w, &cx, MatchPass::Switch).len(), 1);
    }

    #[test]
    fn r22_a_record_binds_one_window() {
        let (mut cx, ids) = with_records(&[("A", vec![empty_record("App", "Title", None)])]);
        let first = window(1, 1, "App", "Title");
        let second = window(1, 2, "App", "Title");
        assert_eq!(cx.rejoin(&first, MatchPass::Switch).len(), 1);
        assert!(cx.rejoin(&second, MatchPass::Switch).is_empty());
        assert!(cx.rejoin(&first, MatchPass::Switch).is_empty());
        assert_eq!(cx.contexts_of(first.wid), vec![ids[0]]);
        assert_eq!(cx.get(ids[0]).unwrap().members.len(), 1);
    }

    #[test]
    fn r22_pinned_record_rejoins_as_pinned() {
        let mut cx = Contexts::new();
        cx.pinned.push(empty_record("Music", "Music", None));
        let w = window(1, 1, "Music", "Music");
        let matches = cx.rejoin(&w, MatchPass::Arrival);
        assert_eq!(steps(&matches), vec![(Slot::Pinned, MatchStep::ExactTitle)]);
        assert!(cx.is_pinned(w.wid));
    }

    #[test]
    fn r22_record_title_follows_its_window() {
        let mut cx = Contexts::new();
        let a = cx.create("A").unwrap();
        let w = window(1, 1, "Chrome", "New Tab");
        cx.add_window(a, &w).unwrap();
        cx.pin(&w);
        cx.title_changed(w.wid, "Docs – Q3 plan");
        assert_eq!(cx.get(a).unwrap().members[0].title, "Docs – Q3 plan");
        assert_eq!(cx.pinned()[0].title, "Docs – Q3 plan");
    }

    #[test]
    fn r22_relaunched_window_with_the_last_title_rejoins_at_step_2() {
        let mut cx = Contexts::new();
        let a = cx.create("A").unwrap();
        let w = window(1, 1, "Chrome", "New Tab");
        cx.add_window(a, &w).unwrap();
        cx.title_changed(w.wid, "Docs – Q3 plan");
        cx.app_terminated(1);
        // After a relaunch the window has a new pid and window server id.
        let relaunched = WindowDesc {
            window_server_id: Some(WindowServerId(5555)),
            ..window(2, 7, "Chrome", "Docs – Q3 plan")
        };
        assert_eq!(
            steps(&cx.rejoin(&relaunched, MatchPass::Arrival)),
            vec![(Slot::Context(a), MatchStep::ExactTitle)]
        );
        assert_eq!(cx.contexts_of(relaunched.wid), vec![a]);
    }

    #[test]
    fn r20_unmatched_new_window_joins_the_active_context() {
        let mut cx = Contexts::new();
        let a = cx.create("A").unwrap();
        let w = window(1, 1, "App", "New");
        assert_eq!(cx.window_appeared(&w, named(a)), Arrival::Joined(a));
        assert_eq!(cx.contexts_of(w.wid), vec![a]);
    }

    #[test]
    fn r20_unmatched_new_window_under_everything_or_unsorted_is_unsorted() {
        let mut cx = Contexts::new();
        cx.create("A").unwrap();
        let w1 = window(1, 1, "App", "One");
        let w2 = window(1, 2, "App", "Two");
        assert_eq!(
            cx.window_appeared(&w1, ContextKey::Everything),
            Arrival::Unsorted
        );
        assert_eq!(cx.window_appeared(&w2, ContextKey::Unsorted), Arrival::Unsorted);
        assert!(cx.is_unsorted(w1.wid));
        assert!(cx.is_unsorted(w2.wid));
    }

    #[test]
    fn r21_matched_window_does_not_join_the_active_context() {
        let (mut cx, ids) = with_records(&[
            ("A", vec![empty_record("App", "Title", None)]),
            ("B", vec![]),
        ]);
        let w = window(1, 1, "App", "Title");
        assert!(matches!(
            cx.window_appeared(&w, named(ids[1])),
            Arrival::Rejoined(_)
        ));
        assert_eq!(cx.contexts_of(w.wid), vec![ids[0]]);
    }

    #[test]
    fn r20_a_repeated_arrival_changes_nothing() {
        let mut cx = Contexts::new();
        let a = cx.create("A").unwrap();
        let b = cx.create("B").unwrap();
        let w = window(1, 1, "App", "Title");
        assert_eq!(cx.window_appeared(&w, named(a)), Arrival::Joined(a));
        assert_eq!(cx.window_appeared(&w, named(a)), Arrival::AlreadyMember);
        assert_eq!(cx.window_appeared(&w, named(b)), Arrival::AlreadyMember);
        assert_eq!(cx.contexts_of(w.wid), vec![a]);
        assert_eq!(cx.get(a).unwrap().members.len(), 1);
    }

    #[test]
    fn r23_pending_records_match_nothing() {
        let mut cx = Contexts::new();
        let a = cx.create("A").unwrap();
        let w = window(1, 1, "App", "Title");
        cx.add_window(a, &w).unwrap();
        cx.window_closed(w.wid);
        assert_eq!(cx.get(a).unwrap().members[0].link, RecordLink::Pending(w.wid));
        assert!(cx.is_unsorted(w.wid));
        let same = window(1, 1, "App", "Title");
        assert!(match_window(&same, &cx, MatchPass::Switch).is_empty());
        let other = window(1, 2, "App", "Title");
        assert!(match_window(&other, &cx, MatchPass::Switch).is_empty());
    }

    #[test]
    fn r23_pending_records_stay_when_the_app_terminates() {
        let mut cx = Contexts::new();
        let a = cx.create("A").unwrap();
        let w = window(1, 1, "App", "Title");
        cx.add_window(a, &w).unwrap();
        cx.pin(&w);
        cx.window_closed(w.wid);
        cx.app_terminated(1);
        assert_eq!(cx.get(a).unwrap().members[0].link, RecordLink::Empty);
        assert_eq!(cx.pinned()[0].link, RecordLink::Empty);
        let relaunched = window(2, 1, "App", "Title");
        assert_eq!(cx.rejoin(&relaunched, MatchPass::Arrival).len(), 2);
    }

    #[test]
    fn r23_open_windows_records_stay_when_the_app_terminates() {
        let mut cx = Contexts::new();
        let a = cx.create("A").unwrap();
        let w = window(1, 1, "App", "Title");
        let other_app = window(2, 1, "Other", "Title");
        cx.add_window(a, &w).unwrap();
        cx.add_window(a, &other_app).unwrap();
        cx.app_terminated(1);
        let members = &cx.get(a).unwrap().members;
        assert_eq!(members.len(), 2);
        assert_eq!(members[0].link, RecordLink::Empty);
        assert_eq!(members[1].link, RecordLink::Live(other_app.wid));
    }

    #[test]
    fn r23_pending_records_go_when_the_app_is_still_running() {
        let mut cx = Contexts::new();
        let a = cx.create("A").unwrap();
        let closed = window(1, 1, "App", "Closed");
        let open = window(1, 2, "App", "Open");
        let other_app = window(2, 1, "Other", "Closed");
        for w in [&closed, &open, &other_app] {
            cx.add_window(a, w).unwrap();
        }
        cx.pin(&closed);
        cx.window_closed(closed.wid);
        cx.window_closed(other_app.wid);
        cx.app_still_running(1);
        let members = &cx.get(a).unwrap().members;
        assert_eq!(
            members.iter().map(|m| m.title.as_str()).collect::<Vec<_>>(),
            vec!["Open", "Closed"]
        );
        assert_eq!(members[1].link, RecordLink::Pending(other_app.wid));
        assert!(cx.pinned().is_empty());
    }

    fn member_of(wid: WindowId, contexts: &[ContextId]) -> SwitchWindow {
        SwitchWindow {
            wid,
            contexts: contexts.to_vec(),
            pinned: false,
            minimized: false,
            untracked: false,
            app_hidden: false,
            unseen_space: false,
            parked: false,
            own: false,
            last_focus: None,
        }
    }

    fn one_screen(active: ContextKey, windows: Vec<SwitchWindow>) -> SwitchInput {
        SwitchInput {
            screens: vec![SwitchScreen { active, windows }],
        }
    }

    const A: ContextId = ContextId(1);
    const B: ContextId = ContextId(2);

    #[test]
    fn r12_r15_switch_parks_exactly_the_windows_that_must_not_show() {
        // One app has a window inside and a window outside the target.
        let plan = plan_switch(&one_screen(
            ContextKey::Named(A),
            vec![
                member_of(wid(1, 1), &[A]),
                member_of(wid(1, 2), &[B]),
                member_of(wid(2, 1), &[A, B]),
                member_of(wid(3, 1), &[]),
            ],
        ));
        assert_eq!(plan.park, vec![wid(1, 2), wid(3, 1)]);
        assert_eq!(plan.unpark, vec![]);
    }

    #[test]
    fn r12_parked_members_are_put_back() {
        let plan = plan_switch(&one_screen(
            ContextKey::Named(B),
            vec![
                SwitchWindow {
                    parked: true,
                    ..member_of(wid(1, 1), &[B])
                },
                SwitchWindow {
                    parked: true,
                    ..member_of(wid(1, 2), &[A])
                },
                member_of(wid(1, 3), &[A]),
            ],
        ));
        assert_eq!(plan.unpark, vec![wid(1, 1)]);
        assert_eq!(plan.park, vec![wid(1, 3)]);
    }

    #[test]
    fn r12_focuses_the_most_recently_focused_member() {
        let plan = plan_switch(&one_screen(
            ContextKey::Named(A),
            vec![
                SwitchWindow {
                    last_focus: Some(3),
                    ..member_of(wid(1, 1), &[A])
                },
                SwitchWindow {
                    last_focus: Some(9),
                    ..member_of(wid(1, 2), &[B])
                },
                SwitchWindow {
                    last_focus: Some(5),
                    parked: true,
                    ..member_of(wid(1, 3), &[A])
                },
                member_of(wid(1, 4), &[A]),
            ],
        ));
        assert_eq!(plan.focus, Some(wid(1, 3)));
    }

    #[test]
    fn r12_focus_never_lands_on_windows_the_user_cant_use() {
        let windows = vec![
            SwitchWindow {
                last_focus: Some(1),
                ..member_of(wid(1, 1), &[A])
            },
            SwitchWindow {
                last_focus: Some(2),
                own: true,
                ..member_of(wid(1, 2), &[A])
            },
            SwitchWindow {
                last_focus: Some(3),
                untracked: true,
                ..member_of(wid(1, 3), &[A])
            },
            SwitchWindow {
                last_focus: Some(4),
                minimized: true,
                ..member_of(wid(1, 4), &[A])
            },
            SwitchWindow {
                last_focus: Some(5),
                app_hidden: true,
                ..member_of(wid(1, 5), &[A])
            },
            SwitchWindow {
                last_focus: Some(6),
                unseen_space: true,
                ..member_of(wid(1, 6), &[A])
            },
        ];
        let plan = plan_switch(&one_screen(ContextKey::Named(A), windows));
        assert_eq!(plan.focus, Some(wid(1, 1)));
        let plan = plan_switch(&one_screen(
            ContextKey::Named(A),
            vec![member_of(wid(1, 2), &[B])],
        ));
        assert_eq!(plan.focus, None);
    }

    #[test]
    fn r13_pinned_windows_show_under_every_context() {
        let pinned = SwitchWindow {
            pinned: true,
            parked: true,
            ..member_of(wid(1, 1), &[])
        };
        for active in [
            ContextKey::Named(A),
            ContextKey::Unsorted,
            ContextKey::Everything,
        ] {
            let plan = plan_switch(&one_screen(active, vec![pinned.clone()]));
            assert_eq!(plan.unpark, vec![wid(1, 1)]);
            assert_eq!(plan.park, vec![]);
        }
    }

    #[test]
    fn r13_unsorted_shows_windows_in_no_context() {
        let plan = plan_switch(&one_screen(
            ContextKey::Unsorted,
            vec![member_of(wid(1, 1), &[]), member_of(wid(1, 2), &[A])],
        ));
        assert_eq!(plan.park, vec![wid(1, 2)]);
    }

    #[test]
    fn r14_never_parks_own_untracked_minimized_hidden_or_unseen_windows() {
        let windows = vec![
            SwitchWindow {
                own: true,
                ..member_of(wid(1, 1), &[])
            },
            SwitchWindow {
                untracked: true,
                ..member_of(wid(1, 2), &[])
            },
            SwitchWindow {
                minimized: true,
                ..member_of(wid(1, 3), &[])
            },
            SwitchWindow {
                app_hidden: true,
                ..member_of(wid(1, 4), &[])
            },
            SwitchWindow {
                unseen_space: true,
                ..member_of(wid(1, 5), &[])
            },
            member_of(wid(1, 6), &[]),
        ];
        let plan = plan_switch(&one_screen(ContextKey::Named(A), windows));
        assert_eq!(plan.park, vec![wid(1, 6)]);
    }

    #[test]
    fn r16_reapplying_the_active_context_parks_windows_that_drifted_in() {
        let plan = plan_switch(&one_screen(
            ContextKey::Named(A),
            vec![
                member_of(wid(1, 1), &[A]),
                SwitchWindow {
                    parked: true,
                    ..member_of(wid(1, 2), &[B])
                },
                member_of(wid(1, 3), &[B]),
            ],
        ));
        assert_eq!(plan.park, vec![wid(1, 3)]);
        assert_eq!(plan.unpark, vec![]);
    }

    #[test]
    fn r7_a_global_switch_changes_every_screen() {
        let input = SwitchInput {
            screens: vec![
                SwitchScreen {
                    active: ContextKey::Named(A),
                    windows: vec![member_of(wid(1, 1), &[A]), member_of(wid(1, 2), &[B])],
                },
                SwitchScreen {
                    active: ContextKey::Named(A),
                    windows: vec![
                        member_of(wid(2, 1), &[B]),
                        SwitchWindow {
                            parked: true,
                            ..member_of(wid(2, 2), &[A])
                        },
                    ],
                },
            ],
        };
        let plan = plan_switch(&input);
        assert_eq!(plan.park, vec![wid(1, 2), wid(2, 1)]);
        assert_eq!(plan.unpark, vec![wid(2, 2)]);
    }

    #[test]
    fn r10_the_active_context_applies_to_a_newly_visible_space() {
        // The screen changed Space; the new Space's windows are planned
        // against the same active context.
        let plan = plan_switch(&one_screen(
            ContextKey::Named(A),
            vec![member_of(wid(4, 1), &[A]), member_of(wid(4, 2), &[])],
        ));
        assert_eq!(plan.park, vec![wid(4, 2)]);
    }

    #[test]
    fn r27_everything_parks_nothing_and_puts_back_every_parked_window() {
        let windows = vec![
            member_of(wid(1, 1), &[A]),
            SwitchWindow {
                parked: true,
                ..member_of(wid(1, 2), &[B])
            },
            SwitchWindow {
                parked: true,
                ..member_of(wid(1, 3), &[])
            },
            SwitchWindow {
                parked: true,
                minimized: true,
                ..member_of(wid(1, 4), &[])
            },
            SwitchWindow {
                parked: true,
                untracked: true,
                ..member_of(wid(1, 5), &[])
            },
        ];
        let plan = plan_switch(&one_screen(ContextKey::Everything, windows));
        assert_eq!(plan.park, vec![]);
        assert_eq!(plan.unpark, vec![wid(1, 2), wid(1, 3), wid(1, 4), wid(1, 5)]);
    }

    #[test]
    fn r20_an_unmatched_new_window_is_never_parked() {
        let mut cx = Contexts::new();
        let a = cx.create("A").unwrap();
        cx.switch_to(named(a)).unwrap();
        let w = window(1, 1, "App", "New");
        cx.window_appeared(&w, cx.active());
        let plan = plan_switch(&one_screen(cx.active(), vec![cx.switch_window(w.wid)]));
        assert_eq!(plan.park, vec![]);
    }

    #[test]
    fn r21_a_rejoined_window_outside_the_active_context_is_parked() {
        let mut cx = Contexts::new();
        let a = cx.create("A").unwrap();
        let b = cx.create("B").unwrap();
        let w = window(1, 1, "App", "Title");
        cx.add_window(a, &w).unwrap();
        cx.app_terminated(1);
        cx.switch_to(named(b)).unwrap();
        let relaunched = window(2, 1, "App", "Title");
        assert!(matches!(
            cx.window_appeared(&relaunched, cx.active()),
            Arrival::Rejoined(_)
        ));
        let plan = plan_switch(&one_screen(cx.active(), vec![cx.switch_window(relaunched.wid)]));
        assert_eq!(plan.park, vec![relaunched.wid]);
    }

    #[test]
    fn switch_window_describes_membership_and_focus() {
        let mut cx = Contexts::new();
        let a = cx.create("A").unwrap();
        let w = window(1, 1, "App", "Title");
        cx.add_window(a, &w).unwrap();
        cx.pin(&w);
        cx.window_focused(w.wid);
        let described = cx.switch_window(w.wid);
        assert_eq!(described.contexts, vec![a]);
        assert!(described.pinned);
        assert_eq!(described.last_focus, Some(1));
    }

    const SPEC_EXAMPLE: &str = r#"{
  "version": 1,
  "next_id": 3,
  "use_seq": 42,
  "contexts": [
    { "id": 1, "name": "Comms", "number": 1, "last_used": 42,
      "members": [
        { "bundle_id": "net.whatsapp.WhatsApp", "app_name": "WhatsApp", "title": "WhatsApp", "window_server_id": 81234 }
      ] }
  ],
  "pinned": [],
  "active": { "global": 1 }
}"#;

    fn round_trip(cx: &Contexts) -> Contexts {
        serde_json::from_str(&serde_json::to_string(cx).unwrap()).unwrap()
    }

    #[test]
    fn contexts_json_round_trips_the_spec_example() {
        let cx: Contexts = serde_json::from_str(SPEC_EXAMPLE).unwrap();
        let comms = cx.by_name("Comms").unwrap();
        assert_eq!(comms.id, ContextId(1));
        assert_eq!(comms.number, Some(1));
        assert_eq!(
            comms.members,
            vec![MemberRecord {
                bundle_id: Some("net.whatsapp.WhatsApp".into()),
                app_name: Some("WhatsApp".into()),
                title: "WhatsApp".into(),
                window_server_id: Some(WindowServerId(81234)),
                link: RecordLink::Empty,
            }]
        );
        assert_eq!(cx.active(), ContextKey::Named(ContextId(1)));
        assert_eq!(cx.next_id, 3);
        assert_eq!(cx.use_seq, 42);
        let written = serde_json::to_value(&cx).unwrap();
        let expected: serde_json::Value = serde_json::from_str(SPEC_EXAMPLE).unwrap();
        assert_eq!(written, expected);
    }

    #[test]
    fn contexts_json_writes_built_in_active_contexts_as_strings() {
        let mut cx = Contexts::new();
        assert_eq!(
            serde_json::to_value(&cx).unwrap()["active"],
            serde_json::json!({ "global": "everything" })
        );
        cx.switch_to(ContextKey::Unsorted).unwrap();
        assert_eq!(
            serde_json::to_value(&cx).unwrap()["active"],
            serde_json::json!({ "global": "unsorted" })
        );
        assert_eq!(round_trip(&cx).active(), ContextKey::Unsorted);
    }

    #[test]
    fn r28_contexts_json_without_a_known_active_context_loads_as_everything() {
        let mut doc: serde_json::Value = serde_json::from_str(SPEC_EXAMPLE).unwrap();
        for active in [
            serde_json::Value::Null,
            serde_json::json!({ "global": 7 }),
            serde_json::json!({ "global": "somewhere" }),
            serde_json::json!({ "per_screen": { "1": 1 } }),
        ] {
            doc["active"] = active;
            let cx: Contexts = serde_json::from_value(doc.clone()).unwrap();
            assert_eq!(cx.active(), ContextKey::Everything);
        }
        doc.as_object_mut().unwrap().remove("active");
        let cx: Contexts = serde_json::from_value(doc).unwrap();
        assert_eq!(cx.active(), ContextKey::Everything);
    }

    #[test]
    fn contexts_json_rejects_other_versions() {
        let mut doc: serde_json::Value = serde_json::from_str(SPEC_EXAMPLE).unwrap();
        doc["version"] = serde_json::json!(2);
        assert!(serde_json::from_value::<Contexts>(doc).is_err());
    }

    #[test]
    fn contexts_json_repairs_ids_and_numbers() {
        let doc = serde_json::json!({
            "version": 1,
            "next_id": 1,
            "use_seq": 0,
            "contexts": [
                { "id": 4, "name": "A", "number": 2, "last_used": 9, "members": [] },
                { "id": 5, "name": "B", "number": 2, "last_used": 0, "members": [] },
                { "id": 6, "name": "C", "number": 12, "last_used": 0, "members": [] },
                { "id": 6, "name": "Duplicate", "number": null, "last_used": 0, "members": [] }
            ],
            "pinned": []
        });
        let mut cx: Contexts = serde_json::from_value(doc).unwrap();
        let numbers: Vec<_> = cx.contexts().iter().map(|c| c.number).collect();
        assert_eq!(numbers, vec![Some(2), None, None]);
        assert_eq!(cx.create("D").unwrap(), ContextId(7));
        cx.switch_to(ContextKey::Named(ContextId(5))).unwrap();
        assert!(
            cx.last_used(ContextKey::Named(ContextId(5)))
                > cx.last_used(ContextKey::Named(ContextId(4)))
        );
    }

    #[test]
    fn r23_records_stay_across_a_restart_without_their_live_windows() {
        let mut cx = Contexts::new();
        let a = cx.create("A").unwrap();
        let open = window(1, 1, "App", "Open");
        let closed = window(1, 2, "App", "Closed");
        cx.add_window(a, &open).unwrap();
        cx.add_window(a, &closed).unwrap();
        cx.pin(&open);
        cx.window_closed(closed.wid);
        let written = serde_json::to_value(&cx).unwrap();
        assert_eq!(
            written["contexts"][0]["members"][0].as_object().unwrap().len(),
            4
        );
        let restored = round_trip(&cx);
        let members = &restored.get(a).unwrap().members;
        assert_eq!(
            members.iter().map(|m| m.title.as_str()).collect::<Vec<_>>(),
            vec!["Open", "Closed"]
        );
        assert!(members.iter().all(|m| m.link == RecordLink::Empty));
        assert_eq!(restored.pinned()[0].link, RecordLink::Empty);
        assert!(restored.contexts_of(open.wid).is_empty());
    }
}
