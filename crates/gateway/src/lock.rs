//! Advisory process locks built on `std::fs::File::try_lock`: the data-dir gateway lock
//! (`<data_dir>/gateway.lock`, one gateway per data dir) and per-plugin locks
//! (`<data_dir>/plugins/<id>.lock`). The kernel releases them when the holder dies.

use std::fmt;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// File name of the data-dir lock.
pub const GATEWAY_LOCK_FILENAME: &str = "gateway.lock";

/// The lock is held by another process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockHeld {
    /// Pid recorded in the lock file by the holder, when readable.
    pub pid: Option<u32>,
    pub path: PathBuf,
}

impl fmt::Display for LockHeld {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.pid {
            Some(pid) => write!(
                f,
                "lock '{}' is held by pid {pid}",
                self.path.display()
            ),
            None => write!(f, "lock '{}' is held by another process", self.path.display()),
        }
    }
}

/// Failure to take a lock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockError {
    Held(LockHeld),
    Io { path: PathBuf, detail: String },
}

impl fmt::Display for LockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Held(h) => h.fmt(f),
            Self::Io { path, detail } => write!(f, "lock '{}': {detail}", path.display()),
        }
    }
}

impl std::error::Error for LockError {}

/// An exclusive advisory lock held for as long as the value lives.
#[derive(Debug)]
pub struct FileLock {
    file: File,
    path: PathBuf,
}

impl FileLock {
    /// Path of the lock file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

/// Takes an exclusive non-blocking lock on `path` (parents created), writing our pid into it.
pub fn acquire_file_lock(path: &Path) -> Result<FileLock, LockError> {
    let io_err = |e: std::io::Error| LockError::Io {
        path: path.to_path_buf(),
        detail: e.to_string(),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(io_err)?;
    }
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(io_err)?;
    match file.try_lock() {
        Ok(()) => {
            file.set_len(0).map_err(io_err)?;
            file.seek(SeekFrom::Start(0)).map_err(io_err)?;
            file.write_all(format!("{}\n", std::process::id()).as_bytes())
                .map_err(io_err)?;
            file.flush().map_err(io_err)?;
            Ok(FileLock {
                file,
                path: path.to_path_buf(),
            })
        }
        Err(TryLockError::WouldBlock) => {
            let mut raw = String::new();
            let _ = file.read_to_string(&mut raw);
            Err(LockError::Held(LockHeld {
                pid: raw.trim().parse().ok(),
                path: path.to_path_buf(),
            }))
        }
        Err(TryLockError::Error(e)) => Err(io_err(e)),
    }
}

/// The data-dir lock: its holder is the one process allowed to host a gateway on that dir.
#[derive(Debug)]
pub struct DataDirLock {
    lock: FileLock,
}

impl DataDirLock {
    /// Path of `gateway.lock`.
    #[must_use]
    pub fn path(&self) -> &Path {
        self.lock.path()
    }
}

/// Takes `<data_dir>/gateway.lock`; the error names the holder pid when another process has it.
pub fn acquire_data_dir_lock(data_dir: &Path) -> Result<DataDirLock, LockError> {
    acquire_file_lock(&data_dir.join(GATEWAY_LOCK_FILENAME)).map(|lock| DataDirLock { lock })
}

/// Takes `<data_dir>/plugins/<id>.lock`.
pub fn acquire_plugin_lock(data_dir: &Path, plugin_id: &str) -> Result<FileLock, LockError> {
    acquire_file_lock(&data_dir.join("plugins").join(format!("{plugin_id}.lock")))
}

/// Returns the pid recorded in `<data_dir>/gateway.lock`, if any (informational only).
#[must_use]
pub fn read_data_dir_lock_pid(data_dir: &Path) -> Option<u32> {
    std::fs::read_to_string(data_dir.join(GATEWAY_LOCK_FILENAME))
        .ok()
        .and_then(|s| s.trim().parse().ok())
}
