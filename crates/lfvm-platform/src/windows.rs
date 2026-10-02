use std::fs::Metadata;
use std::io;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};

use crate::{EntryKind, NameIssue};

const FILE_ATTRIBUTE_OFFLINE: u32 = 0x0000_1000;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
const FILE_ATTRIBUTE_RECALL_ON_OPEN: u32 = 0x0004_0000;
const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;

pub fn classify(meta: &Metadata) -> EntryKind {
    let attrs = meta.file_attributes();
    // OneDrive 等的云占位文件同时带有重解析点属性，需先判断。
    if attrs & (FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS | FILE_ATTRIBUTE_RECALL_ON_OPEN | FILE_ATTRIBUTE_OFFLINE) != 0 {
        return EntryKind::CloudPlaceholder;
    }
    if attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0 || meta.file_type().is_symlink() {
        return EntryKind::Link;
    }
    let ft = meta.file_type();
    if ft.is_dir() {
        EntryKind::Dir
    } else if ft.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    }
}

pub fn file_id(_meta: &Metadata) -> Option<(u64, u64)> {
    None
}

pub fn atomic_replace(tmp: &Path, dst: &Path) -> io::Result<()> {
    // std::fs::rename 在 Windows 上使用 MoveFileExW(MOVEFILE_REPLACE_EXISTING)。
    std::fs::rename(tmp, dst)
}

const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2",
    "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

pub fn validate_component(name: &str) -> Result<(), NameIssue> {
    if let Some(c) =
        name.chars().find(|c| matches!(c, '<' | '>' | ':' | '"' | '\\' | '|' | '?' | '*') || (*c as u32) < 0x20)
    {
        return Err(NameIssue::ForbiddenChar(c));
    }
    if name.ends_with(' ') || name.ends_with('.') {
        return Err(NameIssue::TrailingDotOrSpace);
    }
    let stem = name.split('.').next().unwrap_or(name).trim_end();
    if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(stem)) {
        return Err(NameIssue::ReservedName);
    }
    Ok(())
}

pub fn protected_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for var in ["SystemRoot", "ProgramFiles", "ProgramFiles(x86)", "ProgramData"] {
        if let Some(v) = std::env::var_os(var) {
            dirs.push(PathBuf::from(v));
        }
    }
    dirs
}

pub fn sync_dir(_dir: &Path) -> io::Result<()> {
    Ok(())
}
