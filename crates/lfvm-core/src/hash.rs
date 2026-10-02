//! SHA-256 内容摘要（SRS 1.4：按文件字节判断内容是否完全相同）。

use std::io::{self, Read, Write};
use std::path::Path;

use sha2::{Digest, Sha256};

const BUF: usize = 1 << 20;

/// 读取全部内容并计算摘要；若给出 `sink`，同时把读到的字节写入其中（边复制边计算）。
/// 返回（64 位十六进制摘要, 字节数）。
pub fn hash_reader(mut r: impl Read, mut sink: Option<&mut dyn Write>) -> io::Result<(String, u64)> {
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; BUF];
    let mut total = 0u64;
    loop {
        let n = match r.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        hasher.update(&buf[..n]);
        if let Some(w) = sink.as_deref_mut() {
            w.write_all(&buf[..n])?;
        }
        total += n as u64;
    }
    Ok((format!("{:x}", hasher.finalize()), total))
}

pub fn hash_file(path: &Path) -> io::Result<(String, u64)> {
    hash_reader(std::fs::File::open(path)?, None)
}

pub fn hash_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// 是否为合法的摘要字符串（防止把任意字符串拼进内容对象路径）。
pub fn is_valid_hash(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_digest() {
        assert_eq!(hash_bytes(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(hash_reader(&b"abc"[..], None).unwrap().0, hash_bytes(b"abc"));
    }

    #[test]
    fn copies_while_hashing() {
        let mut out = Vec::new();
        let (h, n) = hash_reader(&b"hello"[..], Some(&mut out)).unwrap();
        assert_eq!(out, b"hello");
        assert_eq!(n, 5);
        assert!(is_valid_hash(&h));
        assert!(!is_valid_hash("../etc/passwd"));
    }
}
