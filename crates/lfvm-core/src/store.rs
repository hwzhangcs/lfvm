//! 内容对象库（SRS 表 4-10）：文件内容按 SHA-256 存放，内容相同的文件只存一份。
//!
//! ```text
//! <data_dir>/projects/<project_id>/objects/ab/ab12…（完整摘要作文件名）
//! <data_dir>/projects/<project_id>/tmp/          写入中的临时文件
//! ```

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::hash::{hash_reader, is_valid_hash};

pub struct ObjectStore {
    objects: PathBuf,
    tmp: PathBuf,
}

/// 一次写入的结果。
#[derive(Debug, Clone)]
pub struct Ingested {
    pub hash: String,
    pub size: u64,
    /// 本次是否新建了对象文件（否则内容已存在，只是复用）。
    pub created: bool,
}

impl ObjectStore {
    pub fn new(project_store_dir: &Path) -> Self {
        Self { objects: project_store_dir.join("objects"), tmp: project_store_dir.join("tmp") }
    }

    pub fn ensure_dirs(&self) -> CoreResult<()> {
        for d in [&self.objects, &self.tmp] {
            std::fs::create_dir_all(d).map_err(|e| CoreError::io(e, d))?;
        }
        Ok(())
    }

    pub fn object_path(&self, hash: &str) -> CoreResult<PathBuf> {
        if !is_valid_hash(hash) {
            return Err(CoreError::new(ErrorCode::Internal, format!("无效的内容摘要：{hash}")));
        }
        Ok(self.objects.join(&hash[..2]).join(hash))
    }

    pub fn contains(&self, hash: &str) -> bool {
        self.object_path(hash).is_ok_and(|p| p.is_file())
    }

    /// 把 `src` 的内容复制进存储，边复制边计算摘要。
    /// 若给出 `expected`，摘要不一致说明文件在扫描后被改写，返回 `ChangedExternally`。
    pub fn ingest(&self, src: &Path, expected: Option<&str>) -> CoreResult<Ingested> {
        let tmp_path = self.tmp.join(format!("{}.part", crate::new_id()));
        let result = (|| {
            let input = File::open(src).map_err(|e| CoreError::io(e, src))?;
            let out = File::create(&tmp_path).map_err(|e| CoreError::io(e, &tmp_path))?;
            let mut w = BufWriter::new(out);
            let (hash, size) = hash_reader(input, Some(&mut w)).map_err(|e| CoreError::io(e, src))?;
            let out = w.into_inner().map_err(|e| CoreError::io(e.into_error(), &tmp_path))?;
            out.sync_all().map_err(|e| CoreError::io(e, &tmp_path))?;
            drop(out);
            if let Some(exp) = expected
                && exp != hash
            {
                return Err(CoreError::new(
                    ErrorCode::ChangedExternally,
                    "文件在保存过程中发生变化，请重新查看后保存",
                )
                .with_path(src));
            }
            let dst = self.object_path(&hash)?;
            if dst.is_file() {
                return Ok(Ingested { hash, size, created: false });
            }
            let parent = dst.parent().expect("对象路径总有上级目录");
            std::fs::create_dir_all(parent).map_err(|e| CoreError::io(e, parent))?;
            std::fs::rename(&tmp_path, &dst).map_err(|e| CoreError::io(e, &dst))?;
            Ok(Ingested { hash, size, created: true })
        })();
        let _ = std::fs::remove_file(&tmp_path);
        result
    }

    /// 写入内存中的内容（测试与小文件使用）。
    pub fn ingest_bytes(&self, bytes: &[u8]) -> CoreResult<Ingested> {
        let tmp_path = self.tmp.join(format!("{}.part", crate::new_id()));
        let mut f = File::create(&tmp_path).map_err(|e| CoreError::io(e, &tmp_path))?;
        f.write_all(bytes).map_err(|e| CoreError::io(e, &tmp_path))?;
        drop(f);
        let r = self.ingest(&tmp_path, None);
        let _ = std::fs::remove_file(&tmp_path);
        r
    }

    pub fn open(&self, hash: &str) -> CoreResult<File> {
        let p = self.object_path(hash)?;
        File::open(&p).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                CoreError::new(ErrorCode::ContentCorrupted, "历史内容缺失").with_path(&p)
            } else {
                CoreError::io(e, &p)
            }
        })
    }

    /// 删除对象，返回释放的字节数（对象不存在时为 0）。
    pub fn remove(&self, hash: &str) -> CoreResult<u64> {
        let p = self.object_path(hash)?;
        let size = match std::fs::metadata(&p) {
            Ok(m) => m.len(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(CoreError::io(e, &p)),
        };
        std::fs::remove_file(&p).map_err(|e| CoreError::io(e, &p))?;
        Ok(size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::hash_bytes;

    #[test]
    fn ingest_dedupes_and_verifies() {
        let dir = tempfile::tempdir().unwrap();
        let store = ObjectStore::new(&dir.path().join("p"));
        store.ensure_dirs().unwrap();
        let src = dir.path().join("a.txt");
        std::fs::write(&src, b"hello").unwrap();

        let a = store.ingest(&src, Some(&hash_bytes(b"hello"))).unwrap();
        assert!(a.created);
        assert_eq!(a.size, 5);
        assert!(store.contains(&a.hash));
        let b = store.ingest(&src, None).unwrap();
        assert!(!b.created);

        let err = store.ingest(&src, Some(&hash_bytes(b"other"))).unwrap_err();
        assert_eq!(err.code, ErrorCode::ChangedExternally);
        // 临时文件不残留
        assert_eq!(std::fs::read_dir(dir.path().join("p/tmp")).unwrap().count(), 0);

        assert_eq!(store.remove(&a.hash).unwrap(), 5);
        assert!(!store.contains(&a.hash));
        assert_eq!(store.remove(&a.hash).unwrap(), 0);
    }

    #[test]
    fn rejects_invalid_hash() {
        let store = ObjectStore::new(Path::new("/nonexistent"));
        assert!(store.object_path("../../etc/passwd").is_err());
    }
}
