//! 中文友好的命题分词模块。
//!
//! 写入 FTS5 索引时用 [`tokenize_for_index`]（精确模式 + HMM）；查询时用
//! [`tokenize_for_query`]（搜索引擎模式，切得更细以提升召回）。两者都过滤掉
//! 纯标点 / 空白 token；查询版本额外把每个 token 包成 FTS5 phrase 字面量并
//! 用 `OR` 连接，以避免 jieba 切出的 token 撞上 FTS5 保留字（OR / AND / NEAR）
//! 或语法字符（`"` `(` `)` `*` `^` `:`）。

use std::sync::OnceLock;

use jieba_rs::Jieba;

/// 全局 jieba 单例。词典加载约 200–500ms，所以应用启动时建议在后台 task 里
/// 主动调一次 [`warm_up`] 把它热起来。
fn jieba() -> &'static Jieba {
    static INSTANCE: OnceLock<Jieba> = OnceLock::new();
    INSTANCE.get_or_init(Jieba::new)
}

/// 主动加载 jieba 词典。`UserModel::spawn` 启动后 spawn 一个 blocking task
/// 调用一次，避免首次写入命题时同步阻塞。
pub fn warm_up() {
    let _ = jieba();
}

/// 入库分词：精确模式 + HMM。输出空格拼接的 token 串，写到 FTS5 表的
/// `tokens` 列；FTS5 默认 `unicode61` tokenizer 按空格切回单个 token 建倒排。
pub fn tokenize_for_index(text: &str) -> String {
    jieba()
        .cut(text, true)
        .into_iter()
        .filter(|tok| is_meaningful(tok))
        .collect::<Vec<_>>()
        .join(" ")
}

/// 查询分词：搜索引擎模式（`cut_for_search`），切得更细。返回拼好的 FTS5
/// MATCH 表达式（OR 连接的 phrase 字面量），如 `"沟通" OR "消息"`。
///
/// 当 query 文本不含任何有意义 token 时返回 `None`——调用方应据此短路返回
/// 空结果，而不是把空串塞给 FTS5（那会报语法错）。
pub fn tokenize_for_query(text: &str) -> Option<String> {
    let phrases: Vec<String> = jieba()
        .cut_for_search(text, true)
        .into_iter()
        .filter(|tok| is_meaningful(tok))
        .map(sanitize_for_match)
        .collect::<Vec<_>>();
    if phrases.is_empty() {
        None
    } else {
        Some(phrases.join(" OR "))
    }
}

/// 一个 token 至少要含一个 Unicode Letter/Number 才算有意义。这把 `，`、空白、
/// `。`、emoji 等纯符号 token 排掉；CJK 汉字归类 `Letter, other (Lo)`，
/// `is_alphanumeric()` 返回 true，所以中文 token 不会被误删。
fn is_meaningful(tok: &str) -> bool {
    tok.chars().any(|c| c.is_alphanumeric())
}

/// 把一个 jieba token 包装成 FTS5 phrase 字面量。phrase 用双引号括起，内部
/// 双引号通过 `""` 转义。这样即便 token 恰好是 `OR` / `AND` / `NEAR` 等 FTS5
/// 关键字、或含 `(` `)` `*` `^` `:` 等语法字符，也只会被当字面量匹配。
fn sanitize_for_match(tok: &str) -> String {
    let escaped = tok.replace('"', "\"\"");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index_tokens(text: &str) -> Vec<String> {
        tokenize_for_index(text)
            .split_whitespace()
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn index_splits_chinese_phrase() {
        let tokens = index_tokens("用户处理事务时倾向于先关注沟通信息");
        // 不锁死具体切法（jieba 词典升级时不能崩），只断言关键词都被切出来。
        // 注意：jieba 精确模式会把"处理事务"切成一个词，不要断言"处理"单独存在。
        for needle in ["用户", "关注", "沟通", "信息"] {
            assert!(
                tokens.iter().any(|t| t == needle),
                "expected token {needle:?} in {tokens:?}"
            );
        }
    }

    #[test]
    fn index_drops_punctuation_and_whitespace() {
        let tokens = index_tokens("用户，沟通。消息！\n打开 WPS  编辑");
        for noise in ["，", "。", "！", " ", "  "] {
            assert!(
                !tokens.iter().any(|t| t == noise),
                "punctuation/whitespace {noise:?} should be filtered: {tokens:?}"
            );
        }
        assert!(tokens.iter().any(|t| t == "沟通"));
        assert!(tokens.iter().any(|t| t == "WPS"));
    }

    #[test]
    fn index_empty_input_returns_empty_string() {
        assert_eq!(tokenize_for_index(""), "");
        assert_eq!(tokenize_for_index("   \n\t"), "");
        assert_eq!(tokenize_for_index("，。！"), "");
    }

    #[test]
    fn query_returns_quoted_or_joined_phrases() {
        let expr = tokenize_for_query("沟通信息").expect("non-empty query");
        assert!(expr.contains(" OR "), "missing OR connector: {expr:?}");
        assert!(expr.contains("\"沟通\""), "missing quoted 沟通: {expr:?}");
        assert!(expr.contains("\"信息\""), "missing quoted 信息: {expr:?}");
    }

    #[test]
    fn query_none_for_empty_or_punctuation_only() {
        assert!(tokenize_for_query("").is_none());
        assert!(tokenize_for_query("   ").is_none());
        assert!(tokenize_for_query("，。！").is_none());
    }

    #[test]
    fn query_handles_mixed_cjk_ascii() {
        let expr = tokenize_for_query("打开 WPS 编辑文档").expect("non-empty query");
        assert!(expr.contains("\"WPS\""), "expected quoted WPS in {expr:?}");
        assert!(
            expr.contains("\"文档\""),
            "expected quoted 文档 in {expr:?}"
        );
    }

    #[test]
    fn sanitize_wraps_in_phrase_quotes() {
        assert_eq!(sanitize_for_match("沟通"), "\"沟通\"");
        assert_eq!(sanitize_for_match("OR"), "\"OR\"");
        assert_eq!(sanitize_for_match("a*b"), "\"a*b\"");
    }

    #[test]
    fn sanitize_doubles_internal_quote() {
        assert_eq!(sanitize_for_match("a\"b"), "\"a\"\"b\"");
    }

    #[test]
    fn is_meaningful_treats_cjk_as_alphanumeric() {
        assert!(is_meaningful("沟"));
        assert!(is_meaningful("a"));
        assert!(is_meaningful("1"));
        assert!(is_meaningful("沟a1"));
        assert!(!is_meaningful("，"));
        assert!(!is_meaningful(" "));
        assert!(!is_meaningful(""));
        assert!(!is_meaningful("！？"));
    }
}
