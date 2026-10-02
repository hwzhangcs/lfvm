//! 通配符匹配：“*” 表示任意个字符，“?” 表示一个字符，匹配整个名称（SRS 3.3.3.4）。
//! 用于系统默认排除（如 “~$*”）和历史文件搜索。

/// 不区分大小写地判断 `text` 是否与 `pattern` 完全匹配。
pub fn wildcard_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    // 经典的贪心回溯算法，时间 O(|p|·|t|)，无递归。
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

pub fn has_wildcard(s: &str) -> bool {
    s.contains(['*', '?'])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches() {
        assert!(wildcard_match("~$*", "~$报告.docx"));
        assert!(!wildcard_match("~$*", "报告.docx"));
        assert!(wildcard_match("thumbs.db", "Thumbs.db"));
        assert!(wildcard_match("*.png", "a.PNG"));
        assert!(wildcard_match("img_??.jpg", "IMG_01.jpg"));
        assert!(!wildcard_match("img_??.jpg", "IMG_001.jpg"));
        assert!(wildcard_match("*a*b*", "xxaxxbxx"));
        assert!(!wildcard_match("*a*b", "xxaxxbxx"));
        assert!(wildcard_match("*", ""));
        assert!(!wildcard_match("?", ""));
    }
}
