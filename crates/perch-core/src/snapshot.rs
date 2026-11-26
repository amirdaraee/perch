//! Copies of other tools' live SQLite databases, kept in Perch's own data
//! folder. Opening a tool's WAL database in place, even read-only, can create
//! or touch its `-shm` file; reading a copy changes nothing of theirs.
//!
//! This is the one module besides `db.rs` and `settings/store.rs` that
//! writes files, and it writes only inside the `cache` folder it is given,
//! which is always under Perch's own data directory. CI checks every other
//! module for writes.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Size and modification time, which change whenever the tool writes.
fn stamp(path: &Path) -> Option<(u64, std::time::SystemTime)> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.len(), m.modified().ok()?))
}

pub fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// Bring `cache/<name>` up to date with `source` and its WAL, copying only
/// when either has changed, and return the copy; `None` when `source` doesn't
/// exist. `source` and its folder are only ever read.
pub fn refresh(source: &Path, cache: &Path) -> Result<Option<PathBuf>> {
    let Some(db_stamp) = stamp(source) else {
        return Ok(None);
    };
    let wal = sibling(source, "-wal");
    let wal_stamp = stamp(&wal);
    let fingerprint = format!("{db_stamp:?} {wal_stamp:?}");

    let name = source.file_name().context("a database file has a name")?;
    std::fs::create_dir_all(cache)
        .with_context(|| format!("could not create {}", cache.display()))?;
    let copy = cache.join(name);
    let marker = sibling(&copy, ".source");
    if copy.exists() && std::fs::read_to_string(&marker).ok().as_deref() == Some(&fingerprint) {
        return Ok(Some(copy));
    }
    // A fresh copy each time: a WAL or index left from an older copy would be
    // replayed onto the new database.
    for stale in [sibling(&copy, "-wal"), sibling(&copy, "-shm"), copy.clone()] {
        let _ = std::fs::remove_file(stale);
    }
    std::fs::copy(source, &copy)?;
    if wal_stamp.is_some() {
        std::fs::copy(&wal, sibling(&copy, "-wal"))?;
    }
    std::fs::write(&marker, fingerprint)?;
    Ok(Some(copy))
}
