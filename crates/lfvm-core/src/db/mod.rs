//! SQLite 访问：连接参数与结构迁移（LFVM-IF-02）。

use std::path::Path;

use rusqlite::Connection;

use crate::error::{CoreError, CoreResult, ErrorCode};

/// 按顺序执行的迁移脚本。`PRAGMA user_version` 记录已执行到第几个。
/// 已发布的脚本不得修改，结构变化只能追加新脚本。
const MIGRATIONS: &[&str] = &[include_str!("../../migrations/0001_init.sql")];

pub fn open(path: &Path) -> CoreResult<Connection> {
    let conn = Connection::open(path).map_err(|e| CoreError::from(e).with_path(path))?;
    configure(&conn)?;
    migrate(&conn)?;
    Ok(conn)
}

#[cfg(test)]
pub fn open_in_memory() -> CoreResult<Connection> {
    let conn = Connection::open_in_memory()?;
    configure(&conn)?;
    migrate(&conn)?;
    Ok(conn)
}

fn configure(conn: &Connection) -> CoreResult<()> {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = FULL;
         PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = 5000;",
    )?;
    Ok(())
}

fn migrate(conn: &Connection) -> CoreResult<()> {
    let current: usize = conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? as usize;
    if current > MIGRATIONS.len() {
        return Err(CoreError::new(
            ErrorCode::Database,
            "历史记录由更新版本的程序创建，请升级本软件后再打开",
        ));
    }
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(current) {
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (i + 1) as i64)?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_apply_and_are_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        drop(open(&path).unwrap());
        let conn = open(&path).unwrap();
        let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v as usize, MIGRATIONS.len());
        let tables: i64 = conn
            .query_row("SELECT count(*) FROM sqlite_master WHERE type = 'table'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(tables, 11);
    }

    #[test]
    fn rejects_newer_schema() {
        let conn = open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", 999).unwrap();
        assert!(migrate(&conn).is_err());
    }
}
