//! Notes ↔ persona conflict pass (memory-system-spec §5).
//!
//! Day-1 design: drop the heavyweight per-paragraph LLM judge from the
//! spec. The persona prompt already requires the model to mirror the
//! authoritative notes into the first section, and the recall layer
//! down-weights global notes that are already in persistent injection
//! — so the realistic conflict surface for v0 is narrow: a note like
//! "我不用 Linear" landing in `notes` AFTER persona was last generated.
//!
//! For those, a cheap heuristic suffices until §15 evaluation shows
//! it isn't enough:
//!
//! 1. Walk every active global note that contains a negation cue
//!    ("不用 X", "别 X", "no longer", "deprecated").
//! 2. For each persona paragraph mentioning the post-cue substring,
//!    annotate the paragraph with a `⚠️ 与你最新声明冲突` marker —
//!    don't delete (deleting risks losing context that's still useful
//!    elsewhere on the line).
//!
//! When evaluation shows this is too crude we plug in the LLM judge
//! described in spec §5.1 — its system prompt is already checked in
//! at `prompts/persona_note_conflict.md` (currently un-`include_str!`'d
//! on purpose; wire it up at the same time you swap the heuristic
//! below for an LLM call). The marker syntax stays the same so the
//! file format doesn't have to change.

use crate::domain::note::Note;

const NEGATION_CUES_ZH: &[&str] = &["不用", "不要", "别用", "别再", "没在用", "不再用"];
const NEGATION_CUES_EN: &[&str] = &[
    "don't use",
    "do not use",
    "no longer",
    "stopped using",
    "deprecated",
];

/// Annotate `persona_md` in place for paragraphs that conflict with
/// `notes`. The output is the same Markdown with conflict markers
/// appended. Returns the new Markdown.
pub fn annotate_conflicts(persona_md: &str, notes: &[Note]) -> String {
    // Build a flat list of "topic tokens" we'll substring-match against
    // each persona line. Tokens shorter than 3 chars/runes are dropped
    // — they're too ambiguous to flag a meaningful conflict.
    let mut tokens: Vec<String> = Vec::new();
    for note in notes {
        for topic in extract_negated_topics(&note.content) {
            for word in topic.split(|c: char| c.is_whitespace() || c == '·' || c == '/') {
                let w = word.trim();
                // CJK chars count as 1 each. require at least 2 chars
                // so single-letter cues don't fire on accidental
                // substrings.
                if w.chars().count() >= 2 && !STOPWORDS.contains(&w) {
                    tokens.push(w.to_lowercase());
                }
            }
        }
    }
    if tokens.is_empty() {
        return persona_md.to_string();
    }
    let mut out = String::with_capacity(persona_md.len() + 64);
    for line in persona_md.lines() {
        out.push_str(line);
        if !line.trim().is_empty() {
            let lc = line.to_lowercase();
            let hit = tokens.iter().find(|t| lc.contains(t.as_str()));
            if let Some(token) = hit {
                out.push_str(&format!(
                    " ⚠️ 与你最新的声明冲突（你已经表示不再使用 \"{token}\"）"
                ));
            }
        }
        out.push('\n');
    }
    out
}

const STOPWORDS: &[&str] = &[
    "the", "a", "an", "and", "or", "use", "using", "used", "with", "of", "to", "in", "on", "for",
    "by", "via", "as", "is", "was", "be",
];

fn extract_negated_topics(content: &str) -> Vec<String> {
    let lc = content.to_lowercase();
    let mut topics: Vec<String> = Vec::new();
    for cue in NEGATION_CUES_ZH.iter().chain(NEGATION_CUES_EN.iter()) {
        let cue_lc = cue.to_lowercase();
        let mut start_byte = 0usize;
        while let Some(pos) = lc[start_byte..].find(&cue_lc) {
            let after_cue_byte = start_byte + pos + cue_lc.len();
            // grab the next up-to-24 chars as the "topic" — stop on
            // ASCII punctuation, Chinese 。, comma, or end of input
            let tail: String = lc[after_cue_byte..]
                .chars()
                .take_while(|c| !c.is_ascii_punctuation() && *c != '。' && *c != '，')
                .take(24)
                .collect();
            let trimmed = tail.trim().to_string();
            if !trimmed.is_empty() {
                topics.push(trimmed);
            }
            start_byte = after_cue_byte;
            if start_byte >= lc.len() {
                break;
            }
        }
    }
    topics
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::note::{NoteScope, NoteSourceType, NoteStatus};
    use chrono::Utc;

    fn n(content: &str) -> Note {
        Note {
            id: "x".into(),
            content: content.into(),
            scope: NoteScope::Global,
            scope_ref: None,
            source_type: NoteSourceType::UserExplicit,
            source_message_id: None,
            source_thread_id: None,
            confidence: 1.0,
            status: NoteStatus::Active,
            superseded_by: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_referenced_at: None,
            expires_at: None,
        }
    }

    #[test]
    fn negation_marker_is_added_when_persona_mentions_disowned_topic() {
        let persona = "## 常用软件与用途\n- linear —— 任务追踪\n";
        let out = annotate_conflicts(persona, &[n("我不用 linear")]);
        assert!(out.contains("⚠️ 与你最新的声明冲突"));
    }

    #[test]
    fn unrelated_persona_lines_pass_through_unchanged() {
        let persona = "## 喜好倾向\n- 写代码偏好简洁\n";
        let out = annotate_conflicts(persona, &[n("我不用 linear")]);
        assert_eq!(out.trim(), persona.trim());
    }

    #[test]
    fn english_negation_cues_work() {
        let persona = "- Jira is your tracker\n";
        let out = annotate_conflicts(persona, &[n("I no longer use jira")]);
        assert!(out.contains("⚠️"));
    }
}
