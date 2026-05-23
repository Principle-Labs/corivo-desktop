//! Egress redact —— 把 PII spans 按用户类目开关替换成占位符。
//!
//! 设计原则（spec §3.1, §5.2, §15.2）：
//! 1. **纯函数**：输入 (text, spans, toggles) → 输出 redacted text。
//!    不读 settings、不查模型、不写状态。`PrivacyFilter::enforce`
//!    是异步壳，本模块是底层。这样单元测试不需要 Mutex / Runtime。
//! 2. **char offset，不是 byte offset**。span 里的 start/end 是字符
//!    位置 —— 中文场景下 byte offset 会把单个字符切成两半。Rust
//!    String slicing 是 byte 索引，所以这一层负责 char→byte 转换。
//! 3. **从尾向头替换**：把 spans 按 start 倒序排，逐个 `replace_range`。
//!    避免前面的替换让后面的 offset 失效。
//! 4. **`secret` 已在 capture 时硬替换**（spec §3.2）；egress 这一层
//!    再次替换是兜底 —— 万一 capture 阶段降级跳过了，secret span
//!    仍能被 enforce 拦下。`redacted_in_storage=true` 的 span 走到
//!    这一步时,实际不会改变文本(原文已经是 placeholder),保持
//!    幂等。

use crate::domain::privacy::{CategoryToggles, PiiSpan};

/// 把 spans 中"用户启用 + 类目命中"的部分替换成 placeholder。
///
/// **返回**: 替换后的字符串。span 数组可空、可乱序、可重叠 ——
/// 函数内部会按 start 排序并丢弃越界的 span（防御性，模型理论上
/// 不会输出越界 span，但出错时也别 panic）。
///
/// **复杂度**: O(n + m·k),n=文本字符数, m=spans 数, k=平均 span 跨度。
/// 当前实现为简单清晰起见每个 span 现算一次 char→byte; 性能上
/// 等到 hot path 实测压力大时再改成单 pass。
pub fn redact(text: &str, spans: &[PiiSpan], toggles: &CategoryToggles) -> String {
    if spans.is_empty() {
        return text.to_string();
    }

    // 1. 过滤：只保留用户启用的类目
    let active: Vec<&PiiSpan> = spans
        .iter()
        .filter(|s| toggles.is_enabled(s.label))
        .filter(|s| s.start < s.end) // 防御：空区间直接丢
        .collect();

    if active.is_empty() {
        return text.to_string();
    }

    // 2. 解重叠 —— 贪心区间调度，优先长 span。
    //    PII 场景偏向"保守": 重叠时宁可多 redact（取更宽的覆盖），
    //    不要漏掉。先按 (length DESC, start ASC) 排序,再逐个加入
    //    "已接受集",任何与已接受集重叠的 candidate 丢弃。
    let mut by_length = active;
    by_length.sort_by(|a, b| {
        let la = a.end - a.start;
        let lb = b.end - b.start;
        lb.cmp(&la).then(a.start.cmp(&b.start))
    });
    let mut accepted: Vec<&PiiSpan> = Vec::with_capacity(by_length.len());
    for span in by_length {
        let overlaps = accepted
            .iter()
            .any(|k| !(span.end <= k.start || span.start >= k.end));
        if !overlaps {
            accepted.push(span);
        }
    }

    // 3. 排序：start 降序 —— 从尾向头替换，让前面 span 的 offset
    //    在替换过程中始终有效。
    let mut filtered = accepted;
    filtered.sort_by(|a, b| b.start.cmp(&a.start));

    // 4. 真替换
    let mut result = text.to_string();
    let total_chars = result.chars().count();
    for span in filtered {
        // 越界:span.end > 文本长度 —— 直接跳过
        if span.end > total_chars {
            continue;
        }
        let byte_start = char_to_byte(&result, span.start);
        let byte_end = char_to_byte(&result, span.end);
        if let (Some(b0), Some(b1)) = (byte_start, byte_end) {
            result.replace_range(b0..b1, span.label.redact_placeholder());
        }
    }

    result
}

/// 把 char index 转成 byte index。`char_idx == text.chars().count()`
/// 时返回 `text.len()`（"末尾"）—— 让 span 的 end 可以指向"最后一
/// 个字符之后"。
fn char_to_byte(text: &str, char_idx: usize) -> Option<usize> {
    if char_idx == 0 {
        return Some(0);
    }
    let mut count = 0;
    for (byte_off, _) in text.char_indices() {
        if count == char_idx {
            return Some(byte_off);
        }
        count += 1;
    }
    if count == char_idx {
        Some(text.len())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::privacy::{CategoryToggles, PiiLabel, PiiSpan};

    fn span(start: usize, end: usize, label: PiiLabel) -> PiiSpan {
        PiiSpan {
            start,
            end,
            label,
            score: 0.95,
            redacted_in_storage: false,
        }
    }

    #[test]
    fn empty_spans_returns_input_unchanged() {
        let out = redact("hello", &[], &CategoryToggles::default());
        assert_eq!(out, "hello");
    }

    #[test]
    fn single_english_person_replaced() {
        // "约一下 John, 邮箱..." —— 找 John (chars 4-8) 替换
        let text = "Meet John soon";
        let out = redact(
            text,
            &[span(5, 9, PiiLabel::PrivatePerson)],
            &CategoryToggles::default(),
        );
        assert_eq!(out, "Meet [人名] soon");
    }

    #[test]
    fn chinese_text_char_offsets_dont_split_codepoints() {
        // "约一下 John，邮箱 a@b.com"
        // 字符位置(0-indexed):
        //   0='约', 1='一', 2='下', 3=' ', 4='J', 5='o', 6='h', 7='n',
        //   8='，', 9='邮', 10='箱', 11=' ', 12='a', 13='@', 14='b',
        //   15='.', 16='c', 17='o', 18='m'
        // chars 4-8 = "John"; chars 12-19 = "a@b.com"
        let text = "约一下 John，邮箱 a@b.com";
        let chars: Vec<char> = text.chars().collect();
        // sanity-check 字符位置（test 写错比代码写错更难调）
        assert_eq!(chars[4..8].iter().collect::<String>(), "John");
        assert_eq!(chars[12..19].iter().collect::<String>(), "a@b.com");

        let out = redact(
            text,
            &[
                span(4, 8, PiiLabel::PrivatePerson),
                span(12, 19, PiiLabel::PrivateEmail),
            ],
            &CategoryToggles::default(),
        );
        assert_eq!(out, "约一下 [人名]，邮箱 [邮箱]");
    }

    #[test]
    fn category_off_keeps_original_text() {
        let mut toggles = CategoryToggles::default();
        toggles.private_person = false;
        let out = redact(
            "Meet John",
            &[span(5, 9, PiiLabel::PrivatePerson)],
            &toggles,
        );
        assert_eq!(out, "Meet John");
    }

    #[test]
    fn secret_redact_always_on_even_if_toggle_false() {
        // CategoryToggles::is_enabled(Secret) 永远返回 true（spec §12.1）。
        let mut toggles = CategoryToggles::default();
        toggles.secret = false;
        let out = redact(
            "key=sk-abcdefgh",
            &[span(4, 15, PiiLabel::Secret)],
            &toggles,
        );
        assert_eq!(out, "key=[REDACTED:secret]");
    }

    #[test]
    fn multiple_spans_descending_order_preserved() {
        // 三个 span,故意乱序输入,看是否正确替换。
        // "Alice (a@x.com) called Bob"
        //  0     6        16     23
        let text = "Alice (a@x.com) called Bob";
        let out = redact(
            text,
            &[
                span(23, 26, PiiLabel::PrivatePerson), // Bob
                span(0, 5, PiiLabel::PrivatePerson),   // Alice
                span(7, 14, PiiLabel::PrivateEmail),   // a@x.com
            ],
            &CategoryToggles::default(),
        );
        assert_eq!(out, "[人名] ([邮箱]) called [人名]");
    }

    #[test]
    fn overlapping_spans_prefer_wider_coverage() {
        // PII 场景偏保守 —— 两个重叠 span 时,更宽的胜出（更激进地
        // redact）。长的 (5,15) 应该赢,短的 (8,12) 被丢弃。
        let text = "0123456789abcdefghij"; // 20 chars
        let out = redact(
            text,
            &[
                span(8, 12, PiiLabel::PrivatePerson),
                span(5, 15, PiiLabel::PrivateEmail),
            ],
            &CategoryToggles::default(),
        );
        assert_eq!(out, "01234[邮箱]fghij");
    }

    #[test]
    fn span_with_end_exceeding_text_is_dropped() {
        // 防御性:模型给了越界 end,直接丢弃,不 panic 也不替换其他 span
        let out = redact(
            "hello",
            &[span(0, 100, PiiLabel::PrivatePerson)],
            &CategoryToggles::default(),
        );
        assert_eq!(out, "hello");
    }

    #[test]
    fn empty_span_range_is_dropped() {
        let out = redact(
            "hello",
            &[span(2, 2, PiiLabel::PrivatePerson)],
            &CategoryToggles::default(),
        );
        assert_eq!(out, "hello");
    }

    #[test]
    fn span_at_text_boundary_works() {
        // start = 0, end = text.chars().count() —— 整段替换
        let text = "John Doe";
        let out = redact(
            text,
            &[span(0, 8, PiiLabel::PrivatePerson)],
            &CategoryToggles::default(),
        );
        assert_eq!(out, "[人名]");
    }

    #[test]
    fn cjk_only_text_replaced_correctly() {
        // 全中文:"张伟今天去了北京"
        //          01234567
        // chars 0-2 = "张伟",chars 5-7 = "北京"
        let text = "张伟今天去了北京";
        let chars: Vec<char> = text.chars().collect();
        assert_eq!(chars[0..2].iter().collect::<String>(), "张伟");
        assert_eq!(chars[6..8].iter().collect::<String>(), "北京");

        let out = redact(
            text,
            &[
                span(0, 2, PiiLabel::PrivatePerson),
                span(6, 8, PiiLabel::PrivateAddress),
            ],
            &CategoryToggles::default(),
        );
        assert_eq!(out, "[人名]今天去了[地址]");
    }

    #[test]
    fn unfiltered_spans_dont_block_later_filtered_ones() {
        // 关掉 URL 类目,URL span 不替换 —— 但 email span 仍然要替换。
        let mut toggles = CategoryToggles::default();
        toggles.private_url = false;
        let text = "see https://x.com and mail a@x.com";
        //              ^^^^^^^^^^^^^^         ^^^^^^^
        //              chars 4-17             chars 27-34
        let out = redact(
            text,
            &[
                span(4, 17, PiiLabel::PrivateUrl),
                span(27, 34, PiiLabel::PrivateEmail),
            ],
            &toggles,
        );
        assert_eq!(out, "see https://x.com and mail [邮箱]");
    }
}
