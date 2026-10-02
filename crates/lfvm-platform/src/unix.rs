use std::fs::Metadata;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use crate::{EntryKind, NameIssue};

pub fn classify(meta: &Metadata) -> EntryKind {
    let ft = meta.file_type();
    if ft.is_symlink() {
        return EntryKind::Link;
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::MetadataExt as _;
        // SF_DATALESS：iCloud 等“仅在云端”的文件，读取会触发下载。
        const SF_DATALESS: u32 = 0x4000_0000;
        if meta.st_flags() & SF_DATALESS != 0 {
            return EntryKind::CloudPlaceholder;
        }
    }
    if ft.is_dir() {
        EntryKind::Dir
    } else if ft.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    }
}

pub fn file_id(meta: &Metadata) -> Option<(u64, u64)> {
    Some((meta.dev(), meta.ino()))
}

pub fn atomic_replace(tmp: &Path, dst: &Path) -> io::Result<()> {
    std::fs::rename(tmp, dst)?;
    if let Some(parent) = dst.parent() {
        sync_dir(parent)?;
    }
    Ok(())
}

pub fn validate_component(name: &str) -> Result<(), NameIssue> {
    #[cfg(target_os = "macos")]
    if name.contains(':') {
        return Err(NameIssue::ForbiddenChar(':'));
    }
    let _ = name;
    Ok(())
}

pub fn protected_dirs() -> Vec<PathBuf> {
    // macOS 的用户临时目录位于 /private/var/folders，因此不能整体保护 /var 或 /private。
    let dirs: &[&str] = if cfg!(target_os = "macos") {
        &[
            "/System",
            "/Library",
            "/Applications",
            "/bin",
            "/sbin",
            "/usr",
            "/dev",
            "/private/etc",
            "/private/var/db",
            "/cores",
        ]
    } else {
        &[
            "/bin",
            "/boot",
            "/dev",
            "/etc",
            "/lib",
            "/lib64",
            "/proc",
            "/root",
            "/run",
            "/sbin",
            "/sys",
            "/usr",
            "/var/lib",
            "/var/log",
            "/var/cache",
        ]
    };
    dirs.iter().map(PathBuf::from).collect()
}

pub fn sync_dir(dir: &Path) -> io::Result<()> {
    std::fs::File::open(dir)?.sync_all()
}
