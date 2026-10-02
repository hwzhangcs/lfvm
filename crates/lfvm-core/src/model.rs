//! 多个模块共用的小型数据类型。

use serde::{Deserialize, Serialize};

/// 版本清单、备份清单中的条目类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum EntryType {
    File,
    Directory,
}

impl EntryType {
    pub fn as_str(self) -> &'static str {
        match self {
            EntryType::File => "file",
            EntryType::Directory => "directory",
        }
    }

    pub fn parse(s: &str) -> Self {
        if s == "directory" { EntryType::Directory } else { EntryType::File }
    }

    pub fn is_dir(self) -> bool {
        self == EntryType::Directory
    }
}
