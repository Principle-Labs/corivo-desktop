//! Render a `<relevant_memory>` block for the prompt
//! (memory-system-spec §6.2 / §7).
//!
//! Groups hits by layer so the model can tell "this is a note the
//! user told me" from "this is a frame I saw on screen" without
//! parsing.

use chrono::{DateTime, Utc};

use super::MemoryItem;

pub fn render_relevant_block(items: &[MemoryItem]) -> Option<String> {
    if items.is_empty() {
        return None;
    }
    let mut notes: Vec<&MemoryItem> = Vec::new();
    let mut frames: Vec<&MemoryItem> = Vec::new();
    let mut messages: Vec<&MemoryItem> = Vec::new();
    for item in items {
        match item {
            MemoryItem::Note { .. } => notes.push(item),
            MemoryItem::Frame { .. } => frames.push(item),
            MemoryItem::Message { .. } => messages.push(item),
        }
    }

    let mut sections: Vec<String> = Vec::new();
    if !notes.is_empty() {
        let mut buf = String::from("## 与你之前告诉我的偏好相关\n");
        for item in notes {
            if let MemoryItem::Note { content, .. } = item {
                buf.push_str(&format!("- {}\n", content.trim()));
            }
        }
        sections.push(buf);
    }
    if !messages.is_empty() {
        let mut buf = String::from("## 之前在对话中聊过\n");
        for item in messages {
            if let MemoryItem::Message { content, ts, .. } = item {
                buf.push_str(&format!("- [{}] {}\n", fmt_when(*ts), content.trim()));
            }
        }
        sections.push(buf);
    }
    if !frames.is_empty() {
        let mut buf = String::from("## 在屏幕上看到过\n");
        for item in frames {
            if let MemoryItem::Frame {
                excerpt,
                ts,
                app_name,
                window_title,
                ..
            } = item
            {
                let app = app_name.as_deref().unwrap_or("?");
                let title = window_title.as_deref().unwrap_or("");
                buf.push_str(&format!(
                    "- [{} · {}{}{}] {}\n",
                    fmt_when(*ts),
                    app,
                    if title.is_empty() { "" } else { " · " },
                    title,
                    excerpt.trim(),
                ));
            }
        }
        sections.push(buf);
    }

    let body = sections.join("\n");
    Some(format!(
        "<relevant_memory>\n<!-- 与本轮对话可能相关的过往记忆，按相关性召回。如果不直接适用，忽略即可。 -->\n{body}</relevant_memory>"
    ))
}

fn fmt_when(ts: DateTime<Utc>) -> String {
    // Coarse local-feel formatting in the user's timezone is overkill
    // for an LLM consumer — keep the canonical UTC slug so the model
    // can parse without ambiguity.
    ts.format("%Y-%m-%d %H:%M").to_string()
}
