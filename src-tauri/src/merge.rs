//! Combines two copies of a vault (or two different vaults) without losing information.
//!
//! The rules, applied to every folder and entry by id:
//! - An entry edited in both copies is merged field by field from the last version they
//!   shared: each copy keeps the fields only it changed, and a field changed in both takes
//!   the most recent value. The versions it replaces go to the entry's history.
//! - Otherwise, the copy that already knows the other's version wins, then the most recent.
//! - Content and location have separate timestamps, so moving an entry in one copy and
//!   editing it in the other keeps both changes.
//! - A deletion wins over versions older than it, and loses against later edits.
//! - Folders with the same name and parent, created separately in each copy, become one.
//! - Identical entries created separately in each copy are kept only once.
//!
//! The result never depends on which copy is "local": merging A into B and B into A gives
//! the same vault, so several Lados windows sharing a file always converge.

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, HashSet};

use chrono::{DateTime, TimeDelta, Utc};
use serde::Serialize;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::models::{CustomIcon, DeletedObject, Entry, EntryContent, EntryRevision, Folder, MAX_HISTORY, Vault};

/// Ids that share a key, split by origin: the ones found only in the local copy and the ones
/// found only in the remote copy.
type Candidates = (Vec<Uuid>, Vec<Uuid>);

/// What changed in the local vault, shown to the user after a merge.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeReport {
    pub entries_added: usize,
    pub entries_updated: usize,
    pub entries_deleted: usize,
    pub conflicts_resolved: usize,
    pub duplicates_skipped: usize,
    pub folders_added: usize,
    pub folders_updated: usize,
    pub folders_deleted: usize,
}

pub struct MergeOutcome {
    pub vault: Vault,
    pub report: MergeReport,
    /// The result differs from the local vault, so the interface has to refresh.
    pub local_changed: bool,
    /// The result differs from the other copy, so that copy has to be rewritten.
    pub remote_outdated: bool,
}

pub fn merge(local: &Vault, remote: &Vault) -> MergeOutcome {
    let mut report = MergeReport::default();
    let mut deleted = collect_deletions(local, remote);

    let aliases = unify_folders(&local.folders, &remote.folders);
    let canonical = |id: Uuid| aliases.get(&id).copied().unwrap_or(id);

    let local_folders = canonical_folders(&local.folders, &canonical);
    let remote_folders = canonical_folders(&remote.folders, &canonical);
    let mut folders = merge_folders(&local_folders, &remote_folders, &mut deleted, &mut report);

    let local_entries = Zeroizing::new(canonical_entries(&local.entries, &canonical));
    let remote_entries = Zeroizing::new(canonical_entries(&remote.entries, &canonical));
    let mut entries = merge_entries(&local_entries, &remote_entries, &mut deleted, &mut report);

    repair_tree(&mut folders, &mut entries);

    let (name, name_changed_at) = match local.name_changed_at.cmp(&remote.name_changed_at) {
        Ordering::Greater => (local.name.clone(), local.name_changed_at),
        Ordering::Less => (remote.name.clone(), remote.name_changed_at),
        Ordering::Equal => (local.name.clone().max(remote.name.clone()), local.name_changed_at),
    };

    let mut vault = Vault {
        version: local.version.max(remote.version),
        id: local.id.min(remote.id),
        name,
        name_changed_at,
        created_at: local.created_at.min(remote.created_at),
        folders,
        entries,
        custom_icons: merge_icons(&local.custom_icons, &remote.custom_icons),
        deleted_objects: deleted
            .into_iter()
            .map(|(id, deleted_at)| DeletedObject { id, deleted_at })
            .collect(),
    };
    normalize(&mut vault);

    let local_copy = Zeroizing::new(normalized(local));
    let remote_copy = Zeroizing::new(normalized(remote));
    MergeOutcome {
        local_changed: vault != *local_copy,
        remote_outdated: vault != *remote_copy,
        vault,
        report,
    }
}

/// Puts every list in a canonical order so two equivalent vaults compare as equal.
fn normalize(vault: &mut Vault) {
    vault.folders.sort_by_key(|f| f.id);
    vault.entries.sort_by_key(|e| e.id);
    vault.custom_icons.sort_by_key(|i| i.id);
    vault.deleted_objects.sort_by_key(|d| d.id);
}

fn normalized(vault: &Vault) -> Vault {
    let mut copy = vault.clone();
    normalize(&mut copy);
    copy
}

fn collect_deletions(local: &Vault, remote: &Vault) -> HashMap<Uuid, DateTime<Utc>> {
    let mut deleted = HashMap::new();
    for object in local.deleted_objects.iter().chain(&remote.deleted_objects) {
        deleted
            .entry(object.id)
            .and_modify(|at: &mut DateTime<Utc>| *at = (*at).max(object.deleted_at))
            .or_insert(object.deleted_at);
    }
    deleted
}

fn is_deleted(deleted: &HashMap<Uuid, DateTime<Utc>>, id: Uuid, last_change: DateTime<Utc>) -> bool {
    deleted.get(&id).is_some_and(|deleted_at| *deleted_at >= last_change)
}

/// Returns `(newest, oldest)`. Ties are broken with `tie` so both copies pick the same winner.
fn newest<'a, T>(
    a: &'a T,
    a_time: DateTime<Utc>,
    b: &'a T,
    b_time: DateTime<Utc>,
    tie: impl FnOnce() -> Ordering,
) -> (&'a T, &'a T) {
    match a_time.cmp(&b_time).then_with(tie) {
        Ordering::Less => (b, a),
        _ => (a, b),
    }
}

// ----- Folders -----

/// Finds folders that exist only in one copy but match a folder that exists only in the other
/// (same parent, same name ignoring case). Both get the smaller of their two ids.
fn unify_folders(local: &[Folder], remote: &[Folder]) -> HashMap<Uuid, Uuid> {
    let local_ids: HashSet<Uuid> = local.iter().map(|f| f.id).collect();
    let remote_ids: HashSet<Uuid> = remote.iter().map(|f| f.id).collect();
    let local_only: Vec<&Folder> = local.iter().filter(|f| !remote_ids.contains(&f.id)).collect();
    let remote_only: Vec<&Folder> = remote.iter().filter(|f| !local_ids.contains(&f.id)).collect();

    let mut aliases = HashMap::new();
    if local_only.is_empty() || remote_only.is_empty() {
        return aliases;
    }

    let local_depths = depths(local);
    let remote_depths = depths(remote);
    let max_depth = local_depths.values().chain(remote_depths.values()).copied().max().unwrap_or(0);

    // Parents are processed before their children, so a child's key already uses the
    // unified id of its parent.
    for depth in 0..=max_depth {
        let key = |folder: &Folder, aliases: &HashMap<Uuid, Uuid>| {
            let parent = folder.parent_id.map(|p| aliases.get(&p).copied().unwrap_or(p));
            (parent, folder.name.trim().to_lowercase())
        };
        let mut groups: HashMap<(Option<Uuid>, String), Candidates> = HashMap::new();
        for folder in local_only.iter().filter(|f| local_depths[&f.id] == depth) {
            groups.entry(key(folder, &aliases)).or_default().0.push(folder.id);
        }
        for folder in remote_only.iter().filter(|f| remote_depths[&f.id] == depth) {
            groups.entry(key(folder, &aliases)).or_default().1.push(folder.id);
        }
        for (_, (mut local_group, mut remote_group)) in groups {
            local_group.sort();
            remote_group.sort();
            for (l, r) in local_group.into_iter().zip(remote_group) {
                let unified = l.min(r);
                aliases.insert(l, unified);
                aliases.insert(r, unified);
            }
        }
    }
    aliases.retain(|from, to| from != to);
    aliases
}

fn depths(folders: &[Folder]) -> HashMap<Uuid, usize> {
    let parents: HashMap<Uuid, Option<Uuid>> = folders.iter().map(|f| (f.id, f.parent_id)).collect();
    folders
        .iter()
        .map(|folder| {
            let mut depth = 0;
            let mut current = folder.parent_id;
            while let Some(parent) = current {
                if depth > folders.len() {
                    break;
                }
                depth += 1;
                current = parents.get(&parent).copied().flatten();
            }
            (folder.id, depth)
        })
        .collect()
}

fn canonical_folders(folders: &[Folder], canonical: &impl Fn(Uuid) -> Uuid) -> Vec<Folder> {
    folders
        .iter()
        .map(|folder| Folder {
            id: canonical(folder.id),
            parent_id: folder.parent_id.map(canonical),
            ..folder.clone()
        })
        .collect()
}

fn merge_folders(
    local: &[Folder],
    remote: &[Folder],
    deleted: &mut HashMap<Uuid, DateTime<Utc>>,
    report: &mut MergeReport,
) -> Vec<Folder> {
    let local_by_id: HashMap<Uuid, &Folder> = local.iter().map(|f| (f.id, f)).collect();
    let remote_by_id: HashMap<Uuid, &Folder> = remote.iter().map(|f| (f.id, f)).collect();
    let ids: BTreeSet<Uuid> = local_by_id.keys().chain(remote_by_id.keys()).copied().collect();

    let mut result = Vec::with_capacity(ids.len());
    for id in ids {
        let mine = local_by_id.get(&id).copied();
        let merged = match (mine, remote_by_id.get(&id).copied()) {
            (Some(l), Some(r)) => merge_folder(l, r),
            (Some(folder), None) | (None, Some(folder)) => folder.clone(),
            (None, None) => continue,
        };
        if is_deleted(deleted, id, merged.last_change()) {
            if mine.is_some() {
                report.folders_deleted += 1;
            }
            continue;
        }
        deleted.remove(&id);
        match mine {
            None => report.folders_added += 1,
            Some(previous) if *previous != merged => report.folders_updated += 1,
            Some(_) => {}
        }
        result.push(merged);
    }
    result
}

fn merge_folder(l: &Folder, r: &Folder) -> Folder {
    let (content, _) = newest(l, l.modified_at, r, r.modified_at, || {
        (&l.name, &l.icon).cmp(&(&r.name, &r.icon))
    });
    let (location, _) = newest(l, l.location_changed_at, r, r.location_changed_at, || {
        l.parent_id.cmp(&r.parent_id)
    });
    Folder {
        id: l.id,
        parent_id: location.parent_id,
        name: content.name.clone(),
        icon: content.icon.clone(),
        created_at: l.created_at.min(r.created_at),
        modified_at: content.modified_at,
        location_changed_at: location.location_changed_at,
    }
}

// ----- Entries -----

fn canonical_entries(entries: &[Entry], canonical: &impl Fn(Uuid) -> Uuid) -> Vec<Entry> {
    entries
        .iter()
        .map(|entry| Entry {
            folder_id: entry.folder_id.map(canonical),
            ..entry.clone()
        })
        .collect()
}

fn merge_entries(
    local: &[Entry],
    remote: &[Entry],
    deleted: &mut HashMap<Uuid, DateTime<Utc>>,
    report: &mut MergeReport,
) -> Vec<Entry> {
    let local_by_id: HashMap<Uuid, &Entry> = local.iter().map(|e| (e.id, e)).collect();
    let remote_by_id: HashMap<Uuid, &Entry> = remote.iter().map(|e| (e.id, e)).collect();
    let ids: BTreeSet<Uuid> = local_by_id.keys().chain(remote_by_id.keys()).copied().collect();

    let mut result = Vec::with_capacity(ids.len());
    for id in ids {
        let mine = local_by_id.get(&id).copied();
        let merged = match (mine, remote_by_id.get(&id).copied()) {
            (Some(l), Some(r)) => {
                let (merged, conflict) = merge_entry(l, r);
                if conflict {
                    report.conflicts_resolved += 1;
                }
                merged
            }
            (Some(entry), None) | (None, Some(entry)) => entry.clone(),
            (None, None) => continue,
        };
        if is_deleted(deleted, id, merged.last_change()) {
            if mine.is_some() {
                report.entries_deleted += 1;
            }
            continue;
        }
        deleted.remove(&id);
        match mine {
            None => report.entries_added += 1,
            Some(previous) if previous.content != merged.content || previous.folder_id != merged.folder_id => {
                report.entries_updated += 1
            }
            Some(_) => {}
        }
        result.push(merged);
    }

    let local_ids: HashSet<Uuid> = local_by_id.keys().copied().collect();
    let remote_ids: HashSet<Uuid> = remote_by_id.keys().copied().collect();
    remove_duplicates(&mut result, &local_ids, &remote_ids, report);
    result
}

/// Returns the merged entry and whether both copies had changed it independently.
fn merge_entry(l: &Entry, r: &Entry) -> (Entry, bool) {
    let (location, _) = newest(l, l.location_changed_at, r, r.location_changed_at, || {
        l.folder_id.cmp(&r.folder_id)
    });
    let (content, modified_at, conflict) = merge_content(l, r);

    let mut history: Vec<EntryRevision> = l.history.iter().chain(&r.history).cloned().collect();
    for side in [l, r] {
        if side.content != content {
            history.push(EntryRevision {
                modified_at: side.modified_at,
                content: side.content.clone(),
            });
        }
    }
    history.retain(|h| !(h.modified_at == modified_at && h.content == content));
    history.sort_by(|a, b| b.modified_at.cmp(&a.modified_at).then_with(|| b.content.cmp(&a.content)));
    history.dedup();
    history.truncate(MAX_HISTORY);

    let merged = Entry {
        id: l.id,
        folder_id: location.folder_id,
        content,
        created_at: l.created_at.min(r.created_at),
        modified_at,
        location_changed_at: location.location_changed_at,
        history,
    };
    (merged, conflict)
}

/// Decides the content of an entry found in both copies. Returns the content, when it was
/// last changed and whether both copies had changed it independently.
fn merge_content(l: &Entry, r: &Entry) -> (EntryContent, DateTime<Utc>, bool) {
    if l.content == r.content {
        return (l.content.clone(), l.modified_at.max(r.modified_at), false);
    }

    // A copy that has the other's current version in its history was edited after seeing
    // it, so it wins even if the clock of its computer is behind.
    let knows = |entry: &Entry, other: &Entry| {
        entry
            .history
            .iter()
            .any(|h| h.modified_at == other.modified_at && h.content == other.content)
    };
    let (winner, loser) = match (knows(l, r), knows(r, l)) {
        (true, false) => return (l.content.clone(), l.modified_at, false),
        (false, true) => return (r.content.clone(), r.modified_at, false),
        (true, true) => {
            let (winner, _) = newest(l, l.modified_at, r, r.modified_at, || l.content.cmp(&r.content));
            return (winner.content.clone(), winner.modified_at, false);
        }
        (false, false) => newest(l, l.modified_at, r, r.modified_at, || l.content.cmp(&r.content)),
    };

    let combined = common_ancestor(l, r).map(|base| combine(&base.content, &winner.content, &loser.content));
    match combined {
        // A version neither copy had yet. It has to be newer than both, so merging it
        // again with either copy keeps it.
        Some(content) if content != winner.content => (content, winner.modified_at + TimeDelta::microseconds(1), true),
        _ => (winner.content.clone(), winner.modified_at, true),
    }
}

/// The newest version both copies keep in their history: the point where they diverged.
fn common_ancestor<'a>(l: &'a Entry, r: &Entry) -> Option<&'a EntryRevision> {
    l.history
        .iter()
        .filter(|revision| r.history.contains(revision))
        .max_by(|a, b| a.modified_at.cmp(&b.modified_at).then_with(|| a.content.cmp(&b.content)))
}

/// Three-way merge of two versions that both evolved from `base`: a field changed in only one
/// of them keeps that change, and a field changed in both takes the value from `preferred`.
fn combine(base: &EntryContent, preferred: &EntryContent, other: &EntryContent) -> EntryContent {
    fn pick<T: Clone + PartialEq>(base: &T, preferred: &T, other: &T) -> T {
        if preferred == base { other.clone() } else { preferred.clone() }
    }
    EntryContent {
        title: pick(&base.title, &preferred.title, &other.title),
        username: pick(&base.username, &preferred.username, &other.username),
        password: pick(&base.password, &preferred.password, &other.password),
        url: pick(&base.url, &preferred.url, &other.url),
        notes: pick(&base.notes, &preferred.notes, &other.notes),
        expiration_date: pick(&base.expiration_date, &preferred.expiration_date, &other.expiration_date),
        icon: pick(&base.icon, &preferred.icon, &other.icon),
        custom_fields: pick(&base.custom_fields, &preferred.custom_fields, &other.custom_fields),
    }
}

/// Two entries with different ids but identical content in the same folder, each coming from
/// a different copy, are the same account added twice. Only the one with the smaller id stays.
fn remove_duplicates(
    entries: &mut Vec<Entry>,
    local_ids: &HashSet<Uuid>,
    remote_ids: &HashSet<Uuid>,
    report: &mut MergeReport,
) {
    let mut groups: HashMap<(Option<Uuid>, &EntryContent), Candidates> = HashMap::new();
    for entry in entries.iter() {
        let in_local = local_ids.contains(&entry.id);
        let in_remote = remote_ids.contains(&entry.id);
        if in_local == in_remote {
            continue;
        }
        let group = groups.entry((entry.folder_id, &entry.content)).or_default();
        if in_local {
            group.0.push(entry.id);
        } else {
            group.1.push(entry.id);
        }
    }

    let mut duplicates = HashSet::new();
    for (_, (mut local_group, mut remote_group)) in groups {
        local_group.sort();
        remote_group.sort();
        for (l, r) in local_group.into_iter().zip(remote_group) {
            duplicates.insert(l.max(r));
            report.duplicates_skipped += 1;
            report.entries_added -= 1;
        }
    }
    entries.retain(|entry| !duplicates.contains(&entry.id));
}

/// Fixes what a merge can break: references to folders that no longer exist and folder
/// cycles created by moving A into B in one copy and B into A in the other.
fn repair_tree(folders: &mut [Folder], entries: &mut [Entry]) {
    let ids: HashSet<Uuid> = folders.iter().map(|f| f.id).collect();
    for folder in folders.iter_mut() {
        if folder.parent_id.is_some_and(|p| !ids.contains(&p)) {
            folder.parent_id = None;
        }
    }

    let mut parents: HashMap<Uuid, Option<Uuid>> = folders.iter().map(|f| (f.id, f.parent_id)).collect();
    for folder in folders.iter_mut() {
        let mut current = folder.parent_id;
        let mut steps = 0;
        while let Some(parent) = current {
            if parent == folder.id {
                folder.parent_id = None;
                parents.insert(folder.id, None);
                break;
            }
            steps += 1;
            if steps > ids.len() {
                break;
            }
            current = parents.get(&parent).copied().flatten();
        }
    }

    for entry in entries.iter_mut() {
        if entry.folder_id.is_some_and(|f| !ids.contains(&f)) {
            entry.folder_id = None;
        }
    }
}

fn merge_icons(local: &[CustomIcon], remote: &[CustomIcon]) -> Vec<CustomIcon> {
    let mut icons = local.to_vec();
    let known: HashSet<Uuid> = local.iter().map(|i| i.id).collect();
    icons.extend(remote.iter().filter(|i| !known.contains(&i.id)).cloned());
    icons
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeDelta;

    fn content(title: &str, password: &str) -> EntryContent {
        EntryContent {
            title: title.into(),
            username: "adria".into(),
            password: password.into(),
            ..Default::default()
        }
    }

    /// A vault with a "Work" folder holding "Mail", plus "Bank" at the top level.
    fn base() -> (Vault, Uuid, Uuid, Uuid) {
        let mut vault = Vault::new("Personal").unwrap();
        let work = vault.add_folder(None, "Work", None).unwrap();
        let mail = vault.add_entry(Some(work), content("Mail", "m-1")).unwrap();
        let bank = vault.add_entry(None, content("Bank", "b-1")).unwrap();
        (vault, work, mail, bank)
    }

    fn shift_entry(vault: &mut Vault, id: Uuid, seconds: i64) {
        let entry = vault.entries.iter_mut().find(|e| e.id == id).unwrap();
        entry.modified_at += TimeDelta::seconds(seconds);
    }

    #[test]
    fn merging_identical_copies_changes_nothing() {
        let (vault, ..) = base();
        let outcome = merge(&vault, &vault.clone());
        assert!(!outcome.local_changed);
        assert!(!outcome.remote_outdated);
        assert_eq!(outcome.report, MergeReport::default());
    }

    #[test]
    fn entries_added_in_each_copy_are_all_kept() {
        let (local, work, ..) = base();
        let mut remote = local.clone();
        let mut local = local;
        local.add_entry(Some(work), content("Server", "s-1")).unwrap();
        remote.add_entry(None, content("Shop", "x-1")).unwrap();

        let outcome = merge(&local, &remote);
        assert_eq!(outcome.vault.entries.len(), 4);
        assert_eq!(outcome.report.entries_added, 1);
        assert!(outcome.local_changed && outcome.remote_outdated);
    }

    #[test]
    fn an_edit_made_in_one_copy_is_applied_without_conflict() {
        let (local, _, mail, _) = base();
        let mut remote = local.clone();
        remote.update_entry(mail, content("Mail", "m-2")).unwrap();

        let outcome = merge(&local, &remote);
        let merged = outcome.vault.entry(mail).unwrap();
        assert_eq!(merged.content.password, "m-2");
        assert_eq!(merged.history[0].content.password, "m-1");
        assert_eq!(outcome.report.entries_updated, 1);
        assert_eq!(outcome.report.conflicts_resolved, 0);
    }

    #[test]
    fn concurrent_edits_keep_the_newest_and_save_the_other_in_history() {
        let (base_vault, _, mail, _) = base();
        let mut local = base_vault.clone();
        let mut remote = base_vault;
        local.update_entry(mail, content("Mail", "local")).unwrap();
        remote.update_entry(mail, content("Mail", "remote")).unwrap();
        shift_entry(&mut remote, mail, 60);

        let outcome = merge(&local, &remote);
        let merged = outcome.vault.entry(mail).unwrap();
        assert_eq!(merged.content.password, "remote");
        let old: Vec<&str> = merged.history.iter().map(|h| h.content.password.as_str()).collect();
        assert_eq!(old, vec!["local", "m-1"]);
        assert_eq!(outcome.report.conflicts_resolved, 1);
    }

    #[test]
    fn edits_to_different_fields_in_each_copy_are_combined() {
        let (base_vault, _, mail, _) = base();
        let mut local = base_vault.clone();
        let mut remote = base_vault;
        let mut renamed = local.entry(mail).unwrap().content.clone();
        renamed.username = "adria.junquera".into();
        local.update_entry(mail, renamed).unwrap();
        remote.update_entry(mail, content("Mail", "m-2")).unwrap();
        shift_entry(&mut remote, mail, 60);

        let outcome = merge(&local, &remote);
        let merged = outcome.vault.entry(mail).unwrap();
        assert_eq!(merged.content.username, "adria.junquera");
        assert_eq!(merged.content.password, "m-2");
        let old: Vec<(&str, &str)> = merged
            .history
            .iter()
            .map(|h| (h.content.username.as_str(), h.content.password.as_str()))
            .collect();
        assert_eq!(old, vec![("adria", "m-2"), ("adria.junquera", "m-1"), ("adria", "m-1")]);
        assert_eq!(outcome.report.conflicts_resolved, 1);

        // Both windows converge on the combined version, and merging it again changes nothing.
        assert_eq!(merge(&remote, &local).vault, outcome.vault);
        assert!(!merge(&outcome.vault, &local).local_changed);
        assert!(!merge(&outcome.vault, &remote).local_changed);
    }

    #[test]
    fn an_edit_made_after_seeing_the_other_version_wins_even_with_a_late_clock() {
        let (local, _, mail, _) = base();
        let mut remote = local.clone();
        remote.update_entry(mail, content("Mail", "m-2")).unwrap();
        // The computer that made the edit has its clock two minutes behind.
        shift_entry(&mut remote, mail, -120);

        let outcome = merge(&local, &remote);
        assert_eq!(outcome.vault.entry(mail).unwrap().content.password, "m-2");
        assert_eq!(outcome.report.conflicts_resolved, 0);
        assert_eq!(outcome.report.entries_updated, 1);
    }

    #[test]
    fn deletions_spread_unless_the_entry_was_edited_afterwards() {
        let (base_vault, _, mail, bank) = base();
        let mut local = base_vault.clone();
        let mut remote = base_vault;
        remote.delete_entry(mail).unwrap();
        remote.delete_entry(bank).unwrap();
        // The local copy edits "Bank" after the other copy deleted it.
        local.update_entry(bank, content("Bank", "b-2")).unwrap();
        shift_entry(&mut local, bank, 60);

        let outcome = merge(&local, &remote);
        assert!(outcome.vault.entry(mail).is_err());
        assert_eq!(outcome.vault.entry(bank).unwrap().content.password, "b-2");
        assert!(!outcome.vault.deleted_objects.iter().any(|d| d.id == bank));
        assert_eq!(outcome.report.entries_deleted, 1);
    }

    #[test]
    fn moving_in_one_copy_and_editing_in_the_other_keeps_both_changes() {
        let (base_vault, work, mail, _) = base();
        let mut local = base_vault.clone();
        let mut remote = base_vault;
        local.move_entry(mail, None).unwrap();
        remote.update_entry(mail, content("Mail", "m-2")).unwrap();

        let merged = merge(&local, &remote).vault;
        let entry = merged.entry(mail).unwrap();
        assert_eq!(entry.folder_id, None);
        assert_eq!(entry.content.password, "m-2");
        assert!(merged.folder(work).is_ok());
    }

    #[test]
    fn folders_created_separately_with_the_same_name_become_one() {
        let mut a = Vault::new("A").unwrap();
        let mut b = Vault::new("B").unwrap();
        let a_work = a.add_folder(None, "Work", None).unwrap();
        let b_work = b.add_folder(None, "work", None).unwrap();
        let a_sub = a.add_folder(Some(a_work), "Servers", None).unwrap();
        let b_sub = b.add_folder(Some(b_work), "Servers", None).unwrap();
        a.add_entry(Some(a_sub), content("SSH", "1")).unwrap();
        b.add_entry(Some(b_sub), content("VPN", "2")).unwrap();

        let ab = merge(&a, &b).vault;
        let ba = merge(&b, &a).vault;
        assert_eq!(ab.folders.len(), 2);
        assert_eq!(ab.folders, ba.folders);
        let servers = ab.folders.iter().find(|f| f.name == "Servers").unwrap().id;
        assert!(ab.entries.iter().all(|e| e.folder_id == Some(servers)));
    }

    #[test]
    fn identical_entries_created_separately_are_not_duplicated() {
        let mut a = Vault::new("A").unwrap();
        let mut b = Vault::new("B").unwrap();
        a.add_entry(None, content("Netflix", "same")).unwrap();
        b.add_entry(None, content("Netflix", "same")).unwrap();

        let outcome = merge(&a, &b);
        assert_eq!(outcome.vault.entries.len(), 1);
        assert_eq!(outcome.report.duplicates_skipped, 1);
        assert_eq!(outcome.report.entries_added, 0);
        assert_eq!(outcome.vault.entries, merge(&b, &a).vault.entries);
    }

    #[test]
    fn entries_added_to_a_folder_deleted_elsewhere_move_to_the_top_level() {
        let (base_vault, work, ..) = base();
        let mut local = base_vault.clone();
        let mut remote = base_vault;
        local.delete_folder(work).unwrap();
        let late = remote.add_entry(Some(work), content("Late", "l-1")).unwrap();
        shift_entry(&mut remote, late, 60);

        let merged = merge(&local, &remote).vault;
        assert!(merged.folder(work).is_err());
        assert_eq!(merged.entry(late).unwrap().folder_id, None);
    }

    #[test]
    fn crossed_folder_moves_do_not_create_a_cycle() {
        let mut base_vault = Vault::new("Test").unwrap();
        let a = base_vault.add_folder(None, "A", None).unwrap();
        let b = base_vault.add_folder(None, "B", None).unwrap();
        let mut local = base_vault.clone();
        let mut remote = base_vault;
        local.move_folder(a, Some(b)).unwrap();
        remote.move_folder(b, Some(a)).unwrap();

        let merged = merge(&local, &remote).vault;
        let top_level = merged.folders.iter().filter(|f| f.parent_id.is_none()).count();
        assert!(top_level >= 1);
    }

    #[test]
    fn merging_is_symmetric_and_stable() {
        let (base_vault, work, mail, bank) = base();
        let mut a = base_vault.clone();
        let mut b = base_vault;
        a.update_entry(mail, content("Mail", "from-a")).unwrap();
        a.add_folder(Some(work), "Shared", None).unwrap();
        b.update_entry(bank, content("Bank", "from-b")).unwrap();
        b.delete_entry(mail).unwrap();
        b.add_folder(Some(work), "shared", None).unwrap();

        let ab = merge(&a, &b);
        let ba = merge(&b, &a);
        assert_eq!(ab.vault, ba.vault);

        let again = merge(&ab.vault, &b);
        assert!(!again.local_changed);
        assert_eq!(again.vault, ab.vault);
    }
}
