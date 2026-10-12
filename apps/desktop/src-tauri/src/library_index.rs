//! The persisted library index: the record set survives the process.
//!
//! # Why this exists
//!
//! Without it, every launch of Lumenfold re-derives the entire library from
//! scratch: walk the folder, open every photo, decode its header, parse its
//! EXIF, publish it, and only then can the grid draw a single tile. For a
//! ten-thousand-photo library that is the entire startup budget spent producing
//! an answer that has not changed since yesterday.
//!
//! So the record set is written to disk when a scan finishes and read back when
//! one starts. The walk still happens — the filesystem is the source of truth and
//! something may have changed since the last run — but the walker consults the
//! persisted records first and skips [`lumenfold_index::probe`] for every file
//! whose size and mtime are unchanged. What survives the restart is not a claim
//! about the folder; it is the expensive half of the answer.
//!
//! # File shape
//!
//! One JSON object per line, with a header line first:
//!
//! ```text
//! {"version":1,"root":"D:\\Photos","count":2}
//! {"id":"...","path":"...","file_name":"a.jpg",...}
//! {"id":"...","path":"...","file_name":"b.jpg",...}
//! ```
//!
//! JSON Lines rather than one big array for two reasons. A 100k-record library
//! is roughly 20 MB; `to_string` would materialise all of it on the heap before
//! a single byte reaches the disk, and `from_str` would need the whole thing
//! live again on the way back. Line-by-line keeps the peak allocation at one
//! record. And a write interrupted by a crash or a full disk leaves a truncated
//! tail instead of a corrupt document — [`load`] treats an unparsable line as
//! the end of the file rather than as a failure, because "no snapshot" and "a
//! snapshot missing its last few records" recover the same way: incrementally
//! rescan.
//!
//! Writes go to a sibling `.tmp` and are renamed into place. A rename within a
//! directory is atomic on every platform this app targets, so a reader never
//! observes a half-written snapshot.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use lumenfold_index::{asset_id_for, AssetRecord};
use tauri::{AppHandle, Manager, Runtime};

use crate::error::LumenError;

/// Bumped whenever the on-disk shape changes incompatibly.
///
/// An unknown version reads as "no snapshot", which is a full incremental
/// rescan — slow, but never wrong. That is the right way for a format bump to
/// fail.
const SNAPSHOT_VERSION: u32 = 1;

/// Header line of a snapshot file.
#[derive(serde::Serialize, serde::Deserialize)]
struct SnapshotHeader {
    version: u32,
    root: String,
    count: usize,
}

/// `<cache dir>/index/<hash-of-root>.jsonl`.
///
/// One file per root so switching between libraries does not have to rewrite
/// the other's index, and so the files can be reasoned about by hand when
/// something looks wrong. The hash is the same [`asset_id_for`] the assets
/// themselves use, which keeps this from needing a second hashing dependency.
fn snapshot_file<R: Runtime>(app: &AppHandle<R>, root: &Path) -> Option<PathBuf> {
    let dir = app.path().app_cache_dir().ok()?.join("index");
    Some(dir.join(format!("{}.jsonl", asset_id_for(root))))
}

/// Reads the persisted records for `root`, or an empty vector if there are none.
///
/// Anything that is not a clean, complete, matching snapshot is "no snapshot":
/// a missing file, an unreadable one, a header from another version, a header
/// naming a different root, or a truncated tail. Startup must not fail because
/// of any of them, and every one of them recovers by rescanning.
pub fn load<R: Runtime>(app: &AppHandle<R>, root: &Path) -> Vec<AssetRecord> {
    let Some(file) = snapshot_file(app, root) else {
        return Vec::new();
    };
    let Ok(opened) = std::fs::File::open(&file) else {
        return Vec::new();
    };
    let mut lines = BufReader::new(opened).lines();

    let Some(header_line) = lines.next() else {
        return Vec::new();
    };
    let Ok(header_line) = header_line else {
        return Vec::new();
    };
    let Ok(header) = serde_json::from_str::<SnapshotHeader>(&header_line) else {
        return Vec::new();
    };
    if header.version != SNAPSHOT_VERSION || header.root != root.to_string_lossy() {
        return Vec::new();
    }

    let mut records = Vec::with_capacity(header.count);
    for line in lines {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        // 一个坏行就到此为止：剩下的行只会更不可信，丢掉整份快照重新扫
        // 和「少认了几张但其余都认」的差别，前者不会把坏数据带进索引。
        let Ok(record) = serde_json::from_str::<AssetRecord>(&line) else {
            break;
        };
        records.push(record);
    }
    records
}

/// Writes `records` as the snapshot for `root`.
///
/// Failure is reported rather than swallowed, because a snapshot that silently
/// never gets written is the exact bug this module exists to fix: the app would
/// look like it persists and quietly rescan everything on every launch.
pub fn save<R: Runtime>(
    app: &AppHandle<R>,
    root: &Path,
    records: &[AssetRecord],
) -> Result<(), LumenError> {
    let Some(file) = snapshot_file(app, root) else {
        return Err(LumenError::io("no cache directory available"));
    };
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            LumenError::io(format!("cannot create {}: {err}", parent.display()))
        })?;
    }
    let temp = file.with_extension("jsonl.tmp");

    let result = (|| -> std::io::Result<()> {
        let out = BufWriter::new(std::fs::File::create(&temp)?);
        let mut out = out;
        let header = SnapshotHeader {
            version: SNAPSHOT_VERSION,
            root: root.to_string_lossy().into_owned(),
            count: records.len(),
        };
        // One record at a time straight into the writer. `to_string` on the whole
        // set would put the entire index on the heap for no reason.
        serde_json::to_writer(&mut out, &header)?;
        out.write_all(b"\n")?;
        for record in records {
            serde_json::to_writer(&mut out, record)?;
            out.write_all(b"\n")?;
        }
        out.flush()?;
        // Rename is atomic within a directory: a concurrent reader sees either
        // the old snapshot or the new one, never a partial write.
        std::fs::rename(&temp, &file)
    })();

    if let Err(err) = result {
        let _ = std::fs::remove_file(&temp);
        return Err(LumenError::io(format!(
            "cannot write {}: {err}",
            file.display()
        )));
    }
    Ok(())
}

/// Records keyed by asset id, which is what the incremental walker expects.
pub fn by_id(records: Vec<AssetRecord>) -> HashMap<String, AssetRecord> {
    records
        .into_iter()
        .map(|record| (record.id.clone(), record))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, name: &str, bytes: u64) -> AssetRecord {
        AssetRecord {
            id: id.to_string(),
            path: PathBuf::from(format!("/lib/{name}")),
            file_name: name.to_string(),
            bytes,
            mtime_secs: 1_700_000_000,
            width: 4000,
            height: 3000,
            taken_at: Some(1_699_999_000),
            tags: lumenfold_index::TAG_CLASSIFIED,
        }
    }

    #[test]
    fn records_round_trip_through_the_line_format() {
        let records = vec![record("a", "a.jpg", 10), record("b", "b.jpg", 20)];
        let mut body = String::new();
        let header = SnapshotHeader {
            version: SNAPSHOT_VERSION,
            root: "/lib".to_string(),
            count: records.len(),
        };
        body.push_str(&serde_json::to_string(&header).unwrap());
        body.push('\n');
        for record in &records {
            body.push_str(&serde_json::to_string(record).unwrap());
            body.push('\n');
        }

        let mut lines = body.lines();
        let parsed: SnapshotHeader = serde_json::from_str(lines.next().unwrap()).unwrap();
        assert_eq!(parsed.version, SNAPSHOT_VERSION);
        assert_eq!(parsed.count, 2);
        let read_back: Vec<AssetRecord> = lines
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(read_back, records);
    }

    /// 快照的价值全在 id 上：复用逻辑和缩略图键都指着它。
    #[test]
    fn by_id_keys_every_record_by_its_own_id() {
        let map = by_id(vec![record("a", "a.jpg", 10), record("b", "b.jpg", 20)]);
        assert_eq!(map.len(), 2);
        assert_eq!(map["a"].file_name, "a.jpg");
        assert_eq!(map["b"].bytes, 20);
    }
}