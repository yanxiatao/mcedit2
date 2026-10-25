//! Conservative, opt-in ONE-REGION transactional persistence for Java MCA/MCR.
//!
//! No GUI Save action yet. This is a library-only, uncompiled implementation
//! until the complete five-round CI batch. Never call it while Minecraft or
//! another editor has the world open. Cross-Region atomicity is NOT claimed.
//!
//! Safety state machine:
//!  1. validate the original and proposed image; refuse any existing marker
//!  2. write + fsync an IMMUTABLE original backup using create_new
//!  3. write + fsync a same-directory staged replacement using create_new
//!  4. write + fsync a recoverable "pending" JSON marker
//!  5. re-read full source and compare the original SHA-256, then rename
//!  6. caller finalizes ONLY after confirming new region, or rolls back
//! A crash while the marker remains is recoverable through rollback_pending.
//! Backup is never deleted by this module, including after rollback.
use crate::region_plan::{verify_candidate, NativeRegionCandidate};
use crate::{WorldError, WorldSummary};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::fs::File; // parent directory fsync is implemented only on Unix
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const MAX_REGION_IMAGE: u64 = 256 * 1024 * 1024;
const MARKER_VERSION: u32 = 1;
fn refuse(text: impl Into<String>) -> WorldError {
    WorldError::Invalid(text.into())
}
fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn suffix(original: &Path, added: &str) -> Result<PathBuf, WorldError> {
    let filename = original
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| refuse("Region filename must be valid UTF-8"))?;
    Ok(original.with_file_name(format!("{filename}{added}")))
}
fn ensure_regular(path: &Path) -> Result<(), WorldError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(refuse(format!(
            "Refusing symlink/nonregular transaction path: {}",
            path.display()
        )));
    }
    Ok(())
}
fn absent(path: &Path) -> Result<(), WorldError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(refuse(format!(
            "Existing transaction path needs explicit recovery: {}",
            path.display()
        ))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
fn append_atomic_marker(path: &Path, bytes: &[u8]) -> Result<(), WorldError> {
    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
/// Atomically replace the current Region file by a fully synced stage.
/// Windows std::fs::rename cannot replace an existing destination. MoveFileExW
/// with REPLACE_EXISTING handles that case without removing the destination
/// first; both paths are in the same original Region directory.
#[cfg(windows)]
fn replace_existing(source: &Path, destination: &Path) -> Result<(), WorldError> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "Kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }
    let src = wide(source);
    let dst = wide(destination);
    // MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH
    let result = unsafe { MoveFileExW(src.as_ptr(), dst.as_ptr(), 0x0000_0001 | 0x0000_0008) };
    if result == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
#[cfg(not(windows))]
fn replace_existing(source: &Path, destination: &Path) -> Result<(), WorldError> {
    fs::rename(source, destination)?;
    Ok(())
}

fn parent_sync(path: &Path) -> Result<(), WorldError> {
    let parent = path
        .parent()
        .ok_or_else(|| refuse("Region has no parent directory"))?;
    #[cfg(unix)]
    {
        File::open(parent)?.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        let _ = parent;
    }
    Ok(())
}
fn expected_region_path(
    world: &WorldSummary,
    dimension: &str,
    rx: i32,
    rz: i32,
    extension: &str,
) -> Result<PathBuf, WorldError> {
    if !matches!(extension, "mca" | "mcr") {
        return Err(refuse(
            "Only original MCA/MCR Region files can be persisted",
        ));
    }
    let source = world
        .dimensions
        .iter()
        .find(|d| d.name == dimension)
        .ok_or_else(|| refuse("Region transaction dimension does not belong to world"))?;
    if source.extension.contains("c.x.z.dat") {
        return Err(refuse(
            "Alpha chunk files are excluded from Region transactions",
        ));
    }
    let base = fs::canonicalize(&world.root)?;
    let directory = fs::canonicalize(&source.region_dir)?;
    if !directory.starts_with(&base) || directory == base {
        return Err(refuse(
            "Original region directory is not nested in source world",
        ));
    }
    if !source.extension.split(", ").any(|s| s == extension) {
        return Err(refuse("Region extension is not indexed in this dimension"));
    }
    let file = directory.join(format!("r.{rx}.{rz}.{extension}"));
    ensure_regular(&file)?;
    if fs::canonicalize(&file)? != file {
        return Err(refuse("Region path is not the canonical original file"));
    }
    let other = directory.join(format!(
        "r.{rx}.{rz}.{}",
        if extension == "mca" { "mcr" } else { "mca" }
    ));
    if other.exists() {
        return Err(refuse("Ambiguous source: MCA and MCR both exist"));
    }
    Ok(file)
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Marker {
    version: u32,
    original_sha256: String,
    replacement_sha256: String,
    original_size: u64,
    replacement_size: u64,
}
#[derive(Debug, Clone)]
pub struct RegionWriteReceipt {
    pub region_path: PathBuf,
    pub backup_path: PathBuf,
    pub marker_path: PathBuf,
    pub original_sha256: String,
    pub replacement_sha256: String,
}
struct Paths {
    region: PathBuf,
    backup: PathBuf,
    staged: PathBuf,
    abandoned: PathBuf,
    restore: PathBuf,
    pending: PathBuf,
    completed: PathBuf,
    rolled_back: PathBuf,
}
fn paths(region: PathBuf, previous: &str, next: &str) -> Result<Paths, WorldError> {
    if previous.len() != 64
        || next.len() != 64
        || !previous
            .bytes()
            .chain(next.bytes())
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(refuse("Transaction SHA-256 identifier malformed"));
    }
    Ok(Paths {
        backup: suffix(&region, &format!(".mcedit2.{previous}.backup"))?,
        staged: suffix(&region, ".mcedit2.stage")?,
        abandoned: suffix(&region, &format!(".mcedit2.{next}.abandoned-stage"))?,
        restore: suffix(&region, ".mcedit2.restore-stage")?,
        pending: suffix(&region, ".mcedit2.pending.json")?,
        completed: suffix(&region, &format!(".mcedit2.{next}.committed.json"))?,
        rolled_back: suffix(&region, &format!(".mcedit2.{next}.rolled-back.json"))?,
        region,
    })
}
fn verify_marker(marker: &Marker) -> Result<(), WorldError> {
    if marker.version != MARKER_VERSION
        || marker.original_size < 8192
        || marker.replacement_size <= marker.original_size
        || marker.original_size > MAX_REGION_IMAGE
        || marker.replacement_size > MAX_REGION_IMAGE
    {
        return Err(refuse("Invalid pending region transaction marker"));
    }
    if marker.original_sha256.len() != 64
        || marker.replacement_sha256.len() != 64
        || !marker
            .original_sha256
            .bytes()
            .chain(marker.replacement_sha256.bytes())
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(refuse("Pending transaction contains malformed digests"));
    }
    Ok(())
}
fn read_pending(region: PathBuf) -> Result<(Marker, Paths), WorldError> {
    let pending = suffix(&region, ".mcedit2.pending.json")?;
    ensure_regular(&pending)?;
    if fs::metadata(&pending)?.len() > 4096 {
        return Err(refuse("Pending marker exceeds 4 KiB"));
    }
    let marker: Marker = serde_json::from_slice(&fs::read(&pending)?)
        .map_err(|e| refuse(format!("Invalid pending region transaction marker: {e}")))?;
    verify_marker(&marker)?;
    let related = paths(region, &marker.original_sha256, &marker.replacement_sha256)?;
    ensure_regular(&related.backup)?;
    let backup = fs::read(&related.backup)?;
    if backup.len() as u64 != marker.original_size || sha(&backup) != marker.original_sha256 {
        return Err(refuse(
            "Original backup is missing, truncated or corrupted; manual recovery required",
        ));
    }
    Ok((marker, related))
}
/// Install one fully verified Region candidate with backup and pending marker.
/// This is a SINGLE-FILE atomic rename, not a multi-region world transaction.
/// The caller must explicitly finalize or roll back; backups are immutable.
pub fn apply_single_region(
    world: &WorldSummary,
    dimension: &str,
    candidate: &NativeRegionCandidate,
) -> Result<RegionWriteReceipt, WorldError> {
    apply_region_with_mode(world, dimension, candidate, false)
}
/// Explicitly confirmed original-NBT-authenticated BE save route.
/// This is the ONLY public write path that can accept existing BE scalar
/// operations, and the candidate still passes full original-byte replay,
/// immutable backup and pending-marker transaction checks.
pub fn apply_authenticated_be_single_region(
    world: &WorldSummary,
    dimension: &str,
    candidate: &NativeRegionCandidate,
    confirmation: &str,
) -> Result<RegionWriteReceipt, WorldError> {
    if confirmation != "SAVE_BE_NBT" || candidate.be_replay.is_empty() {
        return Err(refuse(
            "BE save requires an original BE scalar proof and exact SAVE_BE_NBT confirmation",
        ));
    }
    apply_region_with_mode(world, dimension, candidate, true)
}
fn apply_region_with_mode(
    world: &WorldSummary,
    dimension: &str,
    candidate: &NativeRegionCandidate,
    allow_be: bool,
) -> Result<RegionWriteReceipt, WorldError> {
    if !allow_be && !candidate.be_replay.is_empty() {
        return Err(refuse(
            "BE-aware Region previews need separate SAVE_BE_NBT confirmation",
        ));
    }
    let ext = candidate
        .path
        .extension()
        .and_then(|s| s.to_str())
        .ok_or_else(|| refuse("Region candidate extension is missing"))?;
    let region = expected_region_path(
        world,
        dimension,
        candidate.region_x,
        candidate.region_z,
        ext,
    )?;
    // fs::canonicalize on Windows may add the \\?\ path prefix. The
    // planner keeps the original user-entered absolute path, so lexical
    // equality is not portable. Validate both files as non-symlink regular
    // files, then compare canonical filesystem identities instead.
    ensure_regular(&candidate.path)?;
    if fs::canonicalize(&region)? != fs::canonicalize(&candidate.path)? {
        return Err(refuse(
            "Region candidate references an untrusted/noncanonical path",
        ));
    }
    let original = fs::read(&region)?;
    verify_candidate(&original, candidate)?;
    let next = sha(&candidate.image);
    let files = paths(region, &candidate.original_region_sha256, &next)?;
    for file in [
        &files.pending,
        &files.backup,
        &files.staged,
        &files.abandoned,
        &files.restore,
        &files.completed,
        &files.rolled_back,
    ] {
        absent(file)?;
    }
    let marker = Marker {
        version: MARKER_VERSION,
        original_sha256: candidate.original_region_sha256.clone(),
        replacement_sha256: next.clone(),
        original_size: original.len() as u64,
        replacement_size: candidate.image.len() as u64,
    };
    // Crash before pending marker means original region is still intact and
    // orphaned stage/backup are preserved for manual examination.
    append_atomic_marker(&files.backup, &original)?;
    let json = serde_json::to_vec(&marker)
        .map_err(|e| refuse(format!("Cannot serialize region marker: {e}")))?;
    append_atomic_marker(&files.pending, &json)?;
    parent_sync(&files.region)?;
    append_atomic_marker(&files.staged, &candidate.image)?;
    parent_sync(&files.region)?;
    // Re-check the whole source immediately before replacement. A game or
    // other editor mutating the world during this operation is unsupported.
    ensure_regular(&files.region)?;
    if sha(&fs::read(&files.region)?) != marker.original_sha256 {
        return Err(refuse(
            "Original Region changed during staging; pending marker retained for recovery",
        ));
    }
    replace_existing(&files.staged, &files.region)?;
    parent_sync(&files.region)?;
    Ok(RegionWriteReceipt {
        region_path: files.region,
        backup_path: files.backup,
        marker_path: files.pending,
        original_sha256: marker.original_sha256,
        replacement_sha256: next,
    })
}
/// Confirm a successful one-Region transaction; keep its original backup
/// forever unless the user deletes it explicitly. A failed hash check keeps
/// the pending marker and requires operator recovery.
pub fn finalize_single_region(
    world: &WorldSummary,
    dimension: &str,
    rx: i32,
    rz: i32,
    extension: &str,
) -> Result<RegionWriteReceipt, WorldError> {
    let region = expected_region_path(world, dimension, rx, rz, extension)?;
    let (marker, files) = read_pending(region)?;
    ensure_regular(&files.region)?;
    if sha(&fs::read(&files.region)?) != marker.replacement_sha256 {
        return Err(refuse(
            "Cannot finalize: Region is not exactly the planned replacement",
        ));
    }
    absent(&files.completed)?;
    if files.staged.exists() {
        return Err(refuse("Unresolved transaction stage blocks finalization"));
    }
    fs::rename(&files.pending, &files.completed)?;
    parent_sync(&files.region)?;
    Ok(RegionWriteReceipt {
        region_path: files.region,
        backup_path: files.backup,
        marker_path: files.completed,
        original_sha256: marker.original_sha256,
        replacement_sha256: marker.replacement_sha256,
    })
}
/// Crash recovery ALWAYS selects the ORIGINAL backup (not the edited image).
/// If the current file is neither original nor exact replacement, refuse
/// restoration because another writer may have changed it. Backup is retained.
pub fn rollback_pending_region(
    world: &WorldSummary,
    dimension: &str,
    rx: i32,
    rz: i32,
    extension: &str,
) -> Result<RegionWriteReceipt, WorldError> {
    let region = expected_region_path(world, dimension, rx, rz, extension)?;
    let (marker, files) = read_pending(region)?;
    ensure_regular(&files.region)?;
    let current = sha(&fs::read(&files.region)?);
    if current != marker.original_sha256 && current != marker.replacement_sha256 {
        return Err(refuse(
            "Region changed outside pending transaction; manual recovery required",
        ));
    }
    absent(&files.rolled_back)?;
    if current == marker.replacement_sha256 {
        // An interrupted rollback may leave a verified restore-stage.
        if files.restore.exists() {
            ensure_regular(&files.restore)?;
            if sha(&fs::read(&files.restore)?) != marker.original_sha256 {
                return Err(refuse("Interrupted restoration stage has invalid hash"));
            }
        } else {
            let original = fs::read(&files.backup)?;
            append_atomic_marker(&files.restore, &original)?;
        }
        // One-file replacement of verified backup; never regenerate original
        // region bytes through NBT decoding.
        replace_existing(&files.restore, &files.region)?;
        parent_sync(&files.region)?;
    }
    if sha(&fs::read(&files.region)?) != marker.original_sha256 {
        return Err(refuse(
            "Rollback verification failed; pending marker preserved",
        ));
    }
    // Crash before replacement leaves the write-stage behind. Retain it
    // separately (never execute/delete it) so recovery can still complete.
    if files.staged.exists() {
        ensure_regular(&files.staged)?;
        if sha(&fs::read(&files.staged)?) != marker.replacement_sha256 {
            return Err(refuse(
                "Abandoned write-stage differs from the planned replacement",
            ));
        }
        absent(&files.abandoned)?;
        fs::rename(&files.staged, &files.abandoned)?;
        parent_sync(&files.region)?;
    }
    if files.restore.exists() {
        return Err(refuse(
            "Original restored, but recovery-stage needs manual cleanup",
        ));
    }
    fs::rename(&files.pending, &files.rolled_back)?;
    parent_sync(&files.region)?;
    Ok(RegionWriteReceipt {
        region_path: files.region,
        backup_path: files.backup,
        marker_path: files.rolled_back,
        original_sha256: marker.original_sha256,
        replacement_sha256: marker.replacement_sha256,
    })
}
