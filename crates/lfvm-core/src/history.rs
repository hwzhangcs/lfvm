//! 时间地图与历史版本文件树（SRS 3.3.2.1、3.3.2.3）。

use rusqlite::params;
use serde::Serialize;

use crate::Core;
use crate::content::{SourceRef, check_source};
use crate::error::CoreResult;
use crate::model::EntryType;
use crate::project::load_project;
use crate::store::ObjectStore;

/// 时间地图中的一个版本节点。
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct MapNode {
    pub version_id: String,
    pub seq: u32,
    pub parent_version_id: Option<String>,
    /// 保存时所在的方案；null 表示在默认历史中保存。
    pub origin_scheme_id: Option<String>,
    /// 保存时所在方案的名称（方案已清理时仍保留，供显示）。
    pub origin_scheme_name: Option<String>,
    pub name: String,
    pub note: String,
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub created_at: i64,
    pub file_count: u32,
    pub directory_count: u32,
    pub added: u32,
    pub modified: u32,
    pub deleted: u32,
    /// 内容已清理（占位节点）：不能浏览、比较或恢复。
    pub cleared: bool,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct MapScheme {
    pub scheme_id: String,
    pub name: String,
    pub base_version_id: String,
    pub head_version_id: String,
    #[cfg_attr(feature = "specta", specta(type = specta_typescript::Number))]
    pub created_at: i64,
    /// 已清理的方案不在方案列表中显示，但其版本仍按原方案着色。
    pub cleared: bool,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct TimeMap {
    /// 按保存先后（序号）排列。
    pub nodes: Vec<MapNode>,
    /// 全部方案（含已清理），按创建先后排列。
    pub schemes: Vec<MapScheme>,
    pub default_head: Option<String>,
    pub active_scheme_id: Option<String>,
    /// 版本数超出已测试范围（数据集 D4：500 个版本节点，LFVM-P-13）。
    pub over_scale: bool,
}

/// 历史版本中的一个文件或文件夹。
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct HistoryEntry {
    pub path: String,
    pub entry_type: EntryType,
    #[cfg_attr(feature = "specta", specta(type = Option<specta_typescript::Number>))]
    pub size: Option<i64>,
    pub hash: Option<String>,
    /// 内容对象缺失时为 false（显示“内容损坏”）。
    pub available: bool,
}

impl Core {
    pub fn time_map(&self, project_id: &str) -> CoreResult<TimeMap> {
        let db = self.db();
        let project = load_project(&db, project_id)?;
        let mut stmt = db.prepare(
            "SELECT v.version_id, v.seq, v.parent_version_id, v.origin_scheme_id, s.name, v.name, v.note, v.created_at,
                    v.file_count, v.directory_count, v.added_count, v.modified_count, v.deleted_count, v.payload_state
               FROM versions v LEFT JOIN schemes s ON s.scheme_id = v.origin_scheme_id
              WHERE v.project_id = ?1 ORDER BY v.seq",
        )?;
        let nodes = stmt
            .query_map([project_id], |r| {
                Ok(MapNode {
                    version_id: r.get(0)?,
                    seq: r.get(1)?,
                    parent_version_id: r.get(2)?,
                    origin_scheme_id: r.get(3)?,
                    origin_scheme_name: r.get(4)?,
                    name: r.get(5)?,
                    note: r.get(6)?,
                    created_at: r.get(7)?,
                    file_count: r.get(8)?,
                    directory_count: r.get(9)?,
                    added: r.get(10)?,
                    modified: r.get(11)?,
                    deleted: r.get(12)?,
                    cleared: r.get::<_, String>(13)? == "cleared",
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut stmt = db.prepare(
            "SELECT scheme_id, name, base_version_id, head_version_id, created_at, state
               FROM schemes WHERE project_id = ?1 ORDER BY created_at, scheme_id",
        )?;
        let schemes = stmt
            .query_map([project_id], |r| {
                Ok(MapScheme {
                    scheme_id: r.get(0)?,
                    name: r.get(1)?,
                    base_version_id: r.get(2)?,
                    head_version_id: r.get(3)?,
                    created_at: r.get(4)?,
                    cleared: r.get::<_, String>(5)? == "cleared",
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(TimeMap {
            over_scale: nodes.len() > 500,
            nodes,
            schemes,
            default_head: project.default_head,
            active_scheme_id: project.active_scheme_id,
        })
    }

    /// 版本保存时的完整文件/目录清单（含空目录和零字节文件），按路径排序。
    pub fn version_files(&self, project_id: &str, version_id: &str) -> CoreResult<Vec<HistoryEntry>> {
        self.source_files(project_id, &SourceRef::Version { version_id: version_id.to_owned() })
    }

    /// 版本或安全备份中的全部条目。
    pub fn source_files(&self, project_id: &str, source: &SourceRef) -> CoreResult<Vec<HistoryEntry>> {
        let db = self.db();
        check_source(&db, project_id, source)?;
        let (sql, id) = match source {
            SourceRef::Version { version_id } => (
                "SELECT relative_path, entry_type, size, content_hash FROM version_files WHERE version_id = ?1",
                version_id,
            ),
            SourceRef::Backup { backup_id } => (
                "SELECT relative_path, entry_type, size, content_hash FROM backup_files WHERE backup_id = ?1",
                backup_id,
            ),
        };
        let mut stmt = db.prepare(sql)?;
        let store = ObjectStore::new(&self.project_store_dir(project_id));
        let mut out = stmt
            .query_map(params![id], |r| {
                let hash: Option<String> = r.get(3)?;
                Ok(HistoryEntry {
                    path: r.get(0)?,
                    entry_type: EntryType::parse(&r.get::<_, String>(1)?),
                    size: r.get(2)?,
                    available: hash.as_deref().is_none_or(|h| store.contains(h)),
                    hash,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        out.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(out)
    }
}
