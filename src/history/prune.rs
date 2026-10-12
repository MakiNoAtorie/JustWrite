//! Thinning out old automatic snapshots. Manual, safety and restore snapshots are never removed,
//! and neither is the newest snapshot of any line.

use super::store::{History, Kind};
use crate::jw::{manifest::SnapshotPolicy, time};
use std::{collections::HashSet, io};

const DAY: i64 = 86_400;

/// The automatic snapshots that `policy` would remove at time `now`.
pub fn doomed(history: &History, policy: &SnapshotPolicy, now: i64) -> Vec<String> {
    if !policy.thin {
        return Vec::new();
    }
    let heads: HashSet<&str> = history
        .lines
        .lines
        .iter()
        .filter_map(|l| history.head(&l.name).map(|s| s.id.as_str()))
        .collect();
    let mut keep: HashSet<&str> = HashSet::new();
    // Within a bucket of time on one line, the newest automatic snapshot survives.
    let mut newest: std::collections::HashMap<(String, i64, bool), &str> = std::collections::HashMap::new();
    for s in history.snapshots.iter().filter(|s| s.kind == Kind::Auto) {
        let age_days = (now - s.time) / DAY;
        if age_days < policy.keep_all_days as i64 {
            keep.insert(&s.id);
            continue;
        }
        let (bucket, weekly) = if age_days < policy.keep_daily_days as i64 {
            (time::day(s.time), false)
        } else {
            (time::day(s.time).div_euclid(7), true)
        };
        // Snapshots are oldest first, so a later one replaces an earlier one in its bucket.
        newest.insert((s.line.clone(), bucket, weekly), &s.id);
    }
    keep.extend(newest.values().copied());
    history
        .snapshots
        .iter()
        .filter(|s| s.kind == Kind::Auto && !keep.contains(s.id.as_str()) && !heads.contains(s.id.as_str()))
        .map(|s| s.id.clone())
        .collect()
}

/// Removes them (and any text nothing else uses). Returns how many snapshots went.
pub fn prune(history: &mut History, project: &crate::jw::Project, now: i64) -> io::Result<usize> {
    let ids = doomed(history, &project.manifest.settings.snapshots, now);
    if ids.is_empty() {
        return Ok(0);
    }
    history.remove_snapshots(&project.work, &ids)?;
    Ok(ids.len())
}

/// How far the trimmer thins out: the newest snapshot of each span of time stays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keep {
    Hourly,
    Daily,
    Weekly,
    Nothing,
}
/// What the author asked the trimmer to remove.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trim {
    pub keep: Keep,
    /// Only snapshots at least this many days old.
    pub older_days: u32,
    /// Also the snapshots the author took with a message.
    pub named: bool,
    /// Also safety copies and restores.
    pub safety: bool,
}

/// The snapshots that `trim` would remove at time `now`. The newest snapshot of a line stays.
pub fn trimmed(history: &History, trim: &Trim, now: i64) -> Vec<String> {
    let heads: HashSet<&str> = history
        .lines
        .lines
        .iter()
        .filter_map(|l| history.head(&l.name).map(|s| s.id.as_str()))
        .collect();
    let mut newest: std::collections::HashMap<(&str, i64), &str> = std::collections::HashMap::new();
    let mut gone = Vec::new();
    // Newest first, so the first snapshot met in a span of time is the one that stays. A snapshot
    // that is not up for removal counts too: it is the one kept for its span.
    for s in history.snapshots.iter().rev() {
        let span = match trim.keep {
            Keep::Hourly => Some(s.time.div_euclid(3600)),
            Keep::Daily => Some(time::day(s.time)),
            Keep::Weekly => Some(time::day(s.time).div_euclid(7)),
            Keep::Nothing => None,
        };
        let stays = span.is_some_and(|span| *newest.entry((s.line.as_str(), span)).or_insert(&s.id) == s.id);
        let wanted = match s.kind {
            Kind::Auto => true,
            Kind::Manual => trim.named,
            Kind::Safety | Kind::Restore => trim.safety,
        };
        if wanted && !stays && !heads.contains(s.id.as_str()) && now - s.time >= trim.older_days as i64 * DAY {
            gone.push(s.id.clone());
        }
    }
    gone
}

#[cfg(test)]
mod tests {
    use super::super::store::{SnapshotMeta, testing::fixture};
    use super::*;

    fn meta(id: &str, time: i64, kind: Kind) -> SnapshotMeta {
        SnapshotMeta {
            id: id.into(),
            time,
            message: String::new(),
            kind,
            line: "Main".into(),
            words: 0,
            items: 0,
            signature: id.into(),
            restored_from: None,
        }
    }
    #[test]
    fn old_automatic_snapshots_thin_out_but_manual_ones_stay() {
        let now = 100 * DAY + DAY / 2;
        let mut history = History::default();
        // Today: five auto snapshots, all kept (within the keep-all window).
        for i in 0..5 {
            history.snapshots.push(meta(&format!("today{i}"), now - 3600 * i, Kind::Auto));
        }
        // Ten days ago: three auto snapshots on one day, one manual.
        for i in 0..3 {
            history.snapshots.push(meta(&format!("d10_{i}"), now - 10 * DAY - 3600 * i, Kind::Auto));
        }
        history.snapshots.push(meta("manual10", now - 10 * DAY, Kind::Manual));
        // Sixty days ago: several in one week.
        for i in 0..4 {
            history.snapshots.push(meta(&format!("w60_{i}"), now - 60 * DAY - DAY * i / 2, Kind::Auto));
        }
        history.snapshots.sort_by_key(|s| s.time);
        let policy = SnapshotPolicy { keep_all_days: 2, keep_daily_days: 30, thin: true, ..Default::default() };
        let gone = doomed(&history, &policy, now);
        assert!(gone.iter().all(|id| !id.starts_with("today") && id != "manual10"));
        assert_eq!(gone.iter().filter(|i| i.starts_with("d10_")).count(), 2, "{gone:?}");
        assert_eq!(gone.iter().filter(|i| i.starts_with("w60_")).count(), 3, "{gone:?}");
        let off = SnapshotPolicy { thin: false, ..policy };
        assert!(doomed(&history, &off, now).is_empty());
    }

    #[test]
    fn the_trimmer_keeps_one_per_span_and_spares_named_snapshots_and_heads() {
        let now = 100 * DAY + DAY / 2;
        let mut history = History::default();
        // Four automatic snapshots in one hour two days ago, a named one among them, one today.
        for i in 0..4 {
            history.snapshots.push(meta(&format!("a{i}"), now - 2 * DAY + 60 * i, Kind::Auto));
        }
        history.snapshots.push(meta("named", now - 2 * DAY + 30, Kind::Manual));
        history.snapshots.push(meta("safety", now - 2 * DAY + 90, Kind::Safety));
        history.snapshots.push(meta("head", now, Kind::Auto));
        history.snapshots.sort_by_key(|s| s.time);
        let trim = Trim { keep: Keep::Hourly, older_days: 0, named: false, safety: false };
        let mut gone = trimmed(&history, &trim, now);
        gone.sort();
        assert_eq!(gone, ["a0", "a1", "a2"]);
        assert_eq!(trimmed(&history, &Trim { named: true, safety: true, ..trim }, now).len(), 5, "a3 is the one of its hour");
        let all = Trim { keep: Keep::Nothing, named: true, safety: true, ..trim };
        assert_eq!(trimmed(&history, &all, now).len(), 6, "everything but the head");
        assert!(trimmed(&history, &Trim { older_days: 3, ..all }, now).is_empty());
        assert_eq!(trimmed(&history, &Trim { older_days: 1, ..all }, now).len(), 6);
    }

    #[test]
    fn the_newest_snapshot_of_a_line_is_never_pruned() {
        let now = 1000 * DAY;
        let mut history = History::default();
        history.snapshots.push(meta("old", now - 500 * DAY, Kind::Auto));
        history.snapshots.push(meta("older", now - 501 * DAY, Kind::Auto));
        history.snapshots.sort_by_key(|s| s.time);
        let policy = SnapshotPolicy { keep_all_days: 1, keep_daily_days: 2, thin: true, ..Default::default() };
        let gone = doomed(&history, &policy, now);
        assert!(!gone.contains(&"old".to_string()));
    }

    #[test]
    fn pruning_removes_unused_text_but_keeps_what_remaining_snapshots_need() {
        let mut f = fixture();
        let one = f.ids[0].clone();
        let mut history = History::default();
        let day = DAY;
        let now = 200 * day;
        // Three automatic snapshots a long time ago, each with different text; one manual after.
        let mut ids = Vec::new();
        for i in 0..3 {
            f.project.write_text(&one, &format!("Version {i}.\n")).unwrap();
            ids.push(history.capture(&f.project, Kind::Auto, "", 20 * day + i * 60).unwrap().id);
        }
        f.project.write_text(&one, "Final.\n").unwrap();
        history.capture(&f.project, Kind::Manual, "keep", now - 1).unwrap();
        f.project.manifest.settings.snapshots = SnapshotPolicy { keep_all_days: 1, keep_daily_days: 7, thin: true, ..Default::default() };
        let blobs_before = f.project.work.list("history/blobs").unwrap().len();
        let removed = prune(&mut history, &f.project, now).unwrap();
        assert_eq!(removed, 2);
        assert_eq!(history.snapshots.len(), 2);
        let blobs_after = f.project.work.list("history/blobs").unwrap().len();
        assert!(blobs_after < blobs_before);
        // What survives is still readable, and the index on disk agrees.
        let reloaded = History::load(&f.project.work);
        assert_eq!(reloaded.snapshots.len(), 2);
        let survivor = reloaded.snapshots.iter().find(|s| s.kind == Kind::Auto).unwrap();
        let state = reloaded.state(&f.project.work, &survivor.id).unwrap();
        assert!(state.text(&f.project.work, &one).unwrap().starts_with("Version 2"));
        let manual = reloaded.snapshots.iter().find(|s| s.kind == Kind::Manual).unwrap();
        let state = reloaded.state(&f.project.work, &manual.id).unwrap();
        assert_eq!(state.text(&f.project.work, &one).unwrap(), "Final.\n");
    }
}
