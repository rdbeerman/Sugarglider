// Copyright The Glide Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The journal of parked windows, `parked.json` in the data directory.
//!
//! Each entry holds the frame a window had before Sugarglider parked it. The
//! entry is on disk before the window moves, so the window can be put back
//! after a crash.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use serde::{Deserialize, Serialize};
use tracing::{error, info};

use crate::actor::app::pid_t;
use crate::collections::HashSet;
use crate::sys::window_server::WindowServerId;

const VERSION: u32 = 1;

/// A parked window and the frame it had before it was parked.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JournalEntry {
    pub pid: pid_t,
    #[serde(default)]
    pub bundle_id: Option<String>,
    pub window_server_id: WindowServerId,
    #[serde(default)]
    pub title: String,
    pub frame: JournalFrame,
}

impl JournalEntry {
    fn key(&self) -> (pid_t, WindowServerId) {
        (self.pid, self.window_server_id)
    }
}

/// A frame in the top-left coordinates of the Accessibility API.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct JournalFrame {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl From<CGRect> for JournalFrame {
    fn from(frame: CGRect) -> Self {
        JournalFrame {
            x: frame.origin.x,
            y: frame.origin.y,
            w: frame.size.width,
            h: frame.size.height,
        }
    }
}

impl From<JournalFrame> for CGRect {
    fn from(frame: JournalFrame) -> Self {
        CGRect::new(CGPoint::new(frame.x, frame.y), CGSize::new(frame.w, frame.h))
    }
}

#[derive(Serialize)]
struct FileRef<'a> {
    version: u32,
    entries: &'a [JournalEntry],
}

#[derive(Deserialize)]
struct File {
    version: u32,
    entries: Vec<JournalEntry>,
}

pub struct ParkedJournal {
    /// Where the journal is written. `None` keeps it in memory only.
    path: Option<PathBuf>,
    entries: Vec<JournalEntry>,
    /// Entries read at startup whose windows have not been put back yet.
    unrestored: HashSet<(pid_t, WindowServerId)>,
}

impl ParkedJournal {
    /// Reads the journal at `path`.
    ///
    /// A missing file gives an empty journal. A file that can't be read is
    /// moved to `<name>.unreadable-<unix time>.json` next to it, using `now`,
    /// and the journal starts empty.
    pub fn open(path: PathBuf, now: SystemTime) -> Self {
        let entries = match read(&path) {
            Ok(entries) => entries,
            Err(err) => {
                let aside = unreadable_path(&path, now);
                match fs::rename(&path, &aside) {
                    Ok(()) => error!(
                        ?path,
                        ?aside,
                        "Could not read the parked-window journal ({err}); moved it aside"
                    ),
                    Err(rename_err) => error!(
                        ?path,
                        ?aside,
                        "Could not read the parked-window journal ({err}), \
                         and could not move it aside: {rename_err}"
                    ),
                }
                vec![]
            }
        };
        if !entries.is_empty() {
            info!(count = entries.len(), "Read parked windows from the journal");
        }
        let unrestored = entries.iter().map(JournalEntry::key).collect();
        ParkedJournal {
            path: Some(path),
            entries,
            unrestored,
        }
    }

    /// A journal that is never written to disk.
    pub fn in_memory() -> Self {
        ParkedJournal {
            path: None,
            entries: vec![],
            unrestored: HashSet::default(),
        }
    }

    pub fn entries(&self) -> &[JournalEntry] {
        &self.entries
    }

    pub fn get(&self, pid: pid_t, wsid: WindowServerId) -> Option<&JournalEntry> {
        self.entries.iter().find(|entry| entry.key() == (pid, wsid))
    }

    /// Adds the entries, replacing those for the same windows, and writes the
    /// journal. If the write fails, the journal stays as it was.
    pub fn record(&mut self, new: Vec<JournalEntry>) -> io::Result<()> {
        let mut entries = self.entries.clone();
        for entry in &new {
            entries.retain(|old| old.key() != entry.key());
        }
        entries.extend(new.iter().cloned());
        self.write(&entries)?;
        self.entries = entries;
        for entry in &new {
            self.unrestored.remove(&entry.key());
        }
        Ok(())
    }

    /// Removes the window's entry. Returns whether there was one.
    pub fn remove_window(&mut self, pid: pid_t, wsid: WindowServerId) -> bool {
        self.remove_where(|entry| entry.key() == (pid, wsid))
    }

    /// Removes every entry of the app. Returns whether there were any.
    pub fn remove_app(&mut self, pid: pid_t) -> bool {
        self.remove_where(|entry| entry.pid == pid)
    }

    /// Removes the entries of apps for which `running` returns false.
    pub fn retain_apps(&mut self, running: impl Fn(pid_t) -> bool) -> bool {
        self.remove_where(|entry| !running(entry.pid))
    }

    /// The entries read at startup for the app whose windows have not been put
    /// back yet.
    pub fn unrestored(&self, pid: pid_t) -> Vec<JournalEntry> {
        self.entries
            .iter()
            .filter(|entry| entry.pid == pid && self.unrestored.contains(&entry.key()))
            .cloned()
            .collect()
    }

    /// Notes that the window of an entry read at startup has been put back.
    /// The entry stays until the window's frame is confirmed.
    pub fn mark_restored(&mut self, pid: pid_t, wsid: WindowServerId) {
        self.unrestored.remove(&(pid, wsid));
    }

    /// Removes the entries that `remove` selects and writes the journal if any
    /// were removed. A failed write is logged; the next write retries it.
    fn remove_where(&mut self, remove: impl Fn(&JournalEntry) -> bool) -> bool {
        let before = self.entries.len();
        self.entries.retain(|entry| !remove(entry));
        if self.entries.len() == before {
            return false;
        }
        let entries = &self.entries;
        self.unrestored.retain(|key| entries.iter().any(|entry| entry.key() == *key));
        if let Err(err) = self.write(&self.entries) {
            error!(path = ?self.path, "Could not write the parked-window journal: {err}");
        }
        true
    }

    fn write(&self, entries: &[JournalEntry]) -> io::Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        let dir = path.parent().unwrap_or(Path::new("."));
        fs::create_dir_all(dir)?;
        let json = serde_json::to_string_pretty(&FileRef { version: VERSION, entries })?;
        let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
        tmp.write_all(json.as_bytes())?;
        tmp.persist(path)?;
        Ok(())
    }
}

fn read(path: &Path) -> anyhow::Result<Vec<JournalEntry>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
        Err(err) => return Err(err.into()),
    };
    let file: File = serde_json::from_slice(&bytes)?;
    if file.version != VERSION {
        anyhow::bail!("unknown version {}", file.version);
    }
    Ok(file.entries)
}

fn unreadable_path(path: &Path, now: SystemTime) -> PathBuf {
    let stem = path.file_stem().and_then(|stem| stem.to_str()).unwrap_or("parked");
    let secs = now.duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs());
    path.with_file_name(format!("{stem}.unreadable-{secs}.json"))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use super::{JournalEntry, ParkedJournal};
    use crate::sys::window_server::WindowServerId;

    fn entry(pid: i32, wsid: u32) -> JournalEntry {
        JournalEntry {
            pid,
            bundle_id: Some(format!("com.example.app{pid}")),
            window_server_id: WindowServerId::new(wsid),
            title: format!("Window {wsid}"),
            frame: CGRect::new(CGPoint::new(0., 25.), CGSize::new(1440., 875.)).into(),
        }
    }

    fn now() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_790_000_000)
    }

    fn journal_path(dir: &TempDir) -> PathBuf {
        dir.path().join("parked.json")
    }

    fn file_names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_missing_journal_is_empty_and_is_not_created() {
        let dir = TempDir::new().unwrap();
        let journal = ParkedJournal::open(journal_path(&dir), now());
        assert!(journal.entries().is_empty());
        assert!(file_names(dir.path()).is_empty());
    }

    #[test]
    fn it_writes_the_documented_shape() {
        let dir = TempDir::new().unwrap();
        let mut journal = ParkedJournal::open(journal_path(&dir), now());
        journal
            .record(vec![JournalEntry {
                pid: 812,
                bundle_id: Some("com.google.Chrome".into()),
                window_server_id: WindowServerId::new(9123),
                title: "Docs".into(),
                frame: CGRect::new(CGPoint::new(0., 25.), CGSize::new(1440., 875.)).into(),
            }])
            .unwrap();
        let written: serde_json::Value =
            serde_json::from_slice(&fs::read(journal_path(&dir)).unwrap()).unwrap();
        assert_eq!(
            serde_json::json!({
                "version": 1,
                "entries": [
                    { "pid": 812, "bundle_id": "com.google.Chrome", "window_server_id": 9123,
                      "title": "Docs", "frame": { "x": 0.0, "y": 25.0, "w": 1440.0, "h": 875.0 } }
                ]
            }),
            written
        );
        assert_eq!(vec!["parked.json"], file_names(dir.path()));
    }

    #[test]
    fn entries_read_at_startup_wait_to_be_restored() {
        let dir = TempDir::new().unwrap();
        let mut journal = ParkedJournal::open(journal_path(&dir), now());
        journal.record(vec![entry(1, 10), entry(1, 11), entry(2, 20)]).unwrap();
        assert!(
            journal.unrestored(1).is_empty(),
            "entries written now are not restored"
        );

        let mut journal = ParkedJournal::open(journal_path(&dir), now());
        assert_eq!(vec![entry(1, 10), entry(1, 11)], journal.unrestored(1));
        journal.mark_restored(1, WindowServerId::new(10));
        assert_eq!(vec![entry(1, 11)], journal.unrestored(1));
        assert_eq!(3, journal.entries().len(), "restoring keeps the entry");

        journal.record(vec![entry(2, 20)]).unwrap();
        assert!(
            journal.unrestored(2).is_empty(),
            "a new write replaces the old entry"
        );
    }

    #[test]
    fn r30_a_failed_write_leaves_the_journal_unchanged() {
        let dir = TempDir::new().unwrap();
        let mut journal = ParkedJournal::open(journal_path(&dir), now());
        journal.record(vec![entry(1, 10)]).unwrap();
        let on_disk = fs::read(journal_path(&dir)).unwrap();

        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o555)).unwrap();
        let result = journal.record(vec![entry(1, 11)]);
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();

        assert!(result.is_err());
        assert_eq!(&[entry(1, 10)], journal.entries());
        assert_eq!(on_disk, fs::read(journal_path(&dir)).unwrap());
    }

    #[test]
    fn it_replaces_the_entry_of_a_window_parked_again() {
        let dir = TempDir::new().unwrap();
        let mut journal = ParkedJournal::open(journal_path(&dir), now());
        journal.record(vec![entry(1, 10), entry(1, 11)]).unwrap();
        let mut moved = entry(1, 10);
        moved.frame.x = 500.;
        journal.record(vec![moved.clone()]).unwrap();
        assert_eq!(&[entry(1, 11), moved.clone()], journal.entries());
        assert_eq!(
            &[entry(1, 11), moved],
            ParkedJournal::open(journal_path(&dir), now()).entries()
        );
    }

    #[test]
    fn r31_entries_go_when_their_window_or_app_goes() {
        let dir = TempDir::new().unwrap();
        let mut journal = ParkedJournal::open(journal_path(&dir), now());
        journal
            .record(vec![entry(1, 10), entry(1, 11), entry(2, 20), entry(3, 30)])
            .unwrap();

        assert!(journal.remove_window(1, WindowServerId::new(10)));
        assert!(!journal.remove_window(1, WindowServerId::new(10)));
        assert!(journal.remove_app(2));
        assert!(!journal.remove_app(2));
        assert_eq!(&[entry(1, 11), entry(3, 30)], journal.entries());
        assert_eq!(
            &[entry(1, 11), entry(3, 30)],
            ParkedJournal::open(journal_path(&dir), now()).entries()
        );
    }

    #[test]
    fn r34_it_drops_the_entries_of_apps_that_are_not_running() {
        let dir = TempDir::new().unwrap();
        let mut journal = ParkedJournal::open(journal_path(&dir), now());
        journal.record(vec![entry(1, 10), entry(2, 20)]).unwrap();
        let mut journal = ParkedJournal::open(journal_path(&dir), now());

        assert!(journal.retain_apps(|pid| pid == 1));
        assert!(!journal.retain_apps(|pid| pid == 1));
        assert_eq!(&[entry(1, 10)], journal.entries());
        assert_eq!(vec![entry(1, 10)], journal.unrestored(1));
        assert!(journal.unrestored(2).is_empty());
        assert_eq!(
            &[entry(1, 10)],
            ParkedJournal::open(journal_path(&dir), now()).entries()
        );
    }

    #[test]
    fn removing_nothing_writes_nothing() {
        let dir = TempDir::new().unwrap();
        let mut journal = ParkedJournal::open(journal_path(&dir), now());
        assert!(!journal.remove_app(1));
        assert!(!journal.retain_apps(|_| false));
        assert!(file_names(dir.path()).is_empty());
    }

    #[test]
    fn r34_an_unreadable_journal_is_moved_aside() {
        let dir = TempDir::new().unwrap();
        fs::write(journal_path(&dir), "{ not json").unwrap();
        let mut journal = ParkedJournal::open(journal_path(&dir), now());
        assert!(journal.entries().is_empty());
        assert_eq!(vec!["parked.unreadable-1790000000.json"], file_names(dir.path()));
        assert_eq!(
            "{ not json",
            fs::read_to_string(dir.path().join("parked.unreadable-1790000000.json")).unwrap()
        );

        journal.record(vec![entry(1, 10)]).unwrap();
        assert_eq!(
            &[entry(1, 10)],
            ParkedJournal::open(journal_path(&dir), now()).entries()
        );
    }

    #[test]
    fn r34_a_journal_of_another_version_is_moved_aside() {
        let dir = TempDir::new().unwrap();
        fs::write(journal_path(&dir), r#"{ "version": 2, "entries": [] }"#).unwrap();
        let journal = ParkedJournal::open(journal_path(&dir), now());
        assert!(journal.entries().is_empty());
        assert_eq!(vec!["parked.unreadable-1790000000.json"], file_names(dir.path()));
    }

    #[test]
    fn an_in_memory_journal_writes_nothing() {
        let mut journal = ParkedJournal::in_memory();
        journal.record(vec![entry(1, 10)]).unwrap();
        assert_eq!(&[entry(1, 10)], journal.entries());
        assert!(journal.remove_app(1));
        assert!(journal.entries().is_empty());
    }

    /// Opens a journal file holding `contents`. The file must be moved aside
    /// unchanged, and the journal must start empty and write a new file.
    fn assert_moved_aside(contents: &[u8]) {
        let dir = TempDir::new().unwrap();
        fs::write(journal_path(&dir), contents).unwrap();

        let mut journal = ParkedJournal::open(journal_path(&dir), now());

        assert!(journal.entries().is_empty());
        assert!(journal.unrestored(812).is_empty());
        let aside = dir.path().join("parked.unreadable-1790000000.json");
        assert_eq!(vec!["parked.unreadable-1790000000.json"], file_names(dir.path()));
        assert_eq!(contents, fs::read(&aside).unwrap());

        journal.record(vec![entry(1, 10)]).unwrap();
        let written: serde_json::Value =
            serde_json::from_slice(&fs::read(journal_path(&dir)).unwrap()).unwrap();
        assert_eq!(serde_json::json!(1), written["version"]);
        assert_eq!(
            &[entry(1, 10)],
            ParkedJournal::open(journal_path(&dir), now()).entries()
        );
        assert_eq!(contents, fs::read(&aside).unwrap());
    }

    #[test]
    fn r34_a_journal_cut_off_mid_entry_is_moved_aside_unchanged() {
        assert_moved_aside(
            br#"{ "version": 1, "entries": [ { "pid": 812, "bundle_id": "com.google.Chrome", "window_server_id": 9123, "title": "Docs", "frame": { "x": 0, "y": 25, "w": 14"#,
        );
        assert_moved_aside(br#"{ "version": 1, "entries": ["#);
        assert_moved_aside(br#"{ "version": 1"#);
    }

    #[test]
    fn r34_an_empty_journal_file_is_moved_aside() {
        assert_moved_aside(b"");
    }

    #[test]
    fn r34_journals_of_other_or_missing_versions_are_moved_aside_unchanged() {
        assert_moved_aside(br#"{ "version": 0, "entries": [] }"#);
        assert_moved_aside(
            br#"{ "version": 2, "entries": [ { "pid": 812, "window_server_id": 9123, "frame": { "x": 0, "y": 25, "w": 1440, "h": 875 } } ] }"#,
        );
        assert_moved_aside(br#"{ "entries": [] }"#);
    }

    #[test]
    fn r34_a_journal_whose_entry_has_no_frame_is_moved_aside() {
        assert_moved_aside(
            br#"{ "version": 1, "entries": [ { "pid": 812, "window_server_id": 9123 } ] }"#,
        );
    }

    #[test]
    fn r30_a_write_that_cannot_replace_the_file_changes_nothing_and_leaves_no_temporary_file() {
        let dir = TempDir::new().unwrap();
        let mut journal = ParkedJournal::open(journal_path(&dir), now());
        journal.record(vec![entry(1, 10)]).unwrap();
        // A directory now stands where the journal file was.
        fs::remove_file(journal_path(&dir)).unwrap();
        fs::create_dir(journal_path(&dir)).unwrap();
        fs::write(journal_path(&dir).join("in-the-way"), "").unwrap();

        assert!(journal.record(vec![entry(1, 11), entry(2, 20)]).is_err());

        assert_eq!(&[entry(1, 10)], journal.entries());
        assert_eq!(vec!["parked.json"], file_names(dir.path()));
        fs::remove_dir_all(journal_path(&dir)).unwrap();
        journal.record(vec![entry(2, 20)]).unwrap();
        assert_eq!(
            &[entry(1, 10), entry(2, 20)],
            ParkedJournal::open(journal_path(&dir), now()).entries()
        );
    }

    #[test]
    fn r31_a_removal_that_could_not_be_written_is_written_with_the_next_change() {
        let dir = TempDir::new().unwrap();
        let mut journal = ParkedJournal::open(journal_path(&dir), now());
        journal.record(vec![entry(1, 10), entry(2, 20)]).unwrap();

        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o555)).unwrap();
        let removed = journal.remove_window(1, WindowServerId::new(10));
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();

        assert!(removed);
        assert_eq!(&[entry(2, 20)], journal.entries());
        assert_eq!(
            &[entry(1, 10), entry(2, 20)],
            ParkedJournal::open(journal_path(&dir), now()).entries(),
            "the file keeps the entry until a write succeeds"
        );
        journal.record(vec![entry(3, 30)]).unwrap();
        assert_eq!(
            &[entry(2, 20), entry(3, 30)],
            ParkedJournal::open(journal_path(&dir), now()).entries()
        );
    }
}
