//! Shared output format for AX + OCR text extraction (spec §六).
//!
//! Both extraction paths emit plaintext in reading order, with a small set
//! of high-signal role tags so the downstream summarizer / FTS index can
//! tell titles / buttons / tabs / code blocks apart from body text.
//!
//! Format choice rationale lives in `docs/ax-ocr-spec.md` Q9.

/// High-signal role tags that survive into the plaintext output.
///
/// Everything else (`AXStaticText`, `AXGroup`, etc.) goes in untagged.
#[allow(dead_code)] // wired into ax_extractor in Phase 2
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleTag {
    Title,
    Button,
    Tab,
    Code,
}

impl RoleTag {
    #[allow(dead_code)] // wired into ax_extractor in Phase 2
    pub fn as_marker(self) -> &'static str {
        match self {
            Self::Title => "[TITLE]",
            Self::Button => "[BUTTON]",
            Self::Tab => "[TAB]",
            Self::Code => "[CODE]",
        }
    }
}

/// One observation produced by Apple Vision OCR. The bounding box is in
/// Vision's normalized image coordinates (origin at the **bottom-left**, axes
/// in [0, 1]), which is why higher `midy` means the text sits closer to the
/// **top** of the visible image.
#[derive(Debug, Clone)]
pub struct TextObservation {
    pub text: String,
    /// Horizontal midpoint in [0, 1].
    pub midx: f64,
    /// Vertical midpoint in [0, 1]. Bigger = nearer the top of the image.
    pub midy: f64,
    /// Bounding-box height in [0, 1]. Used for line-bucket sizing.
    pub height: f64,
}

/// Stitch a flat list of OCR observations into reading-order plaintext.
///
/// Algorithm:
/// 1. Sort top-to-bottom (`midy` descending — Vision's Y axis points up).
/// 2. Bucket into lines: an observation joins the previous line when its
///    `midy` is within ½ of the rolling-mean text height of that line.
/// 3. Within each line, sort left-to-right (`midx` ascending), join with `" "`.
/// 4. Lines join with `"\n"`.
///
/// Multi-column layouts (IDE side panel + editor) are **not** column-split in
/// Phase 1 — the simple banded sort produces "column 1 row 1, column 2 row 1,
/// column 1 row 2, …" which is suboptimal but legible. Column splitting can
/// be added in Phase 2 polish if dogfood reveals it as a real problem.
pub fn stitch_observations(mut items: Vec<TextObservation>) -> String {
    if items.is_empty() {
        return String::new();
    }

    items.sort_by(|a, b| {
        b.midy
            .partial_cmp(&a.midy)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut lines: Vec<Vec<TextObservation>> = Vec::new();
    for item in items {
        let attach_to_last = lines.last().map_or(false, |last_line| {
            let line_height_mean: f64 =
                last_line.iter().map(|o| o.height).sum::<f64>() / last_line.len() as f64;
            let threshold = (line_height_mean * 0.5).max(0.005);
            let line_y = last_line.first().map(|o| o.midy).unwrap_or(item.midy);
            (line_y - item.midy).abs() <= threshold
        });
        if attach_to_last {
            lines.last_mut().expect("lines.last() was Some").push(item);
        } else {
            lines.push(vec![item]);
        }
    }

    lines
        .into_iter()
        .map(|mut line| {
            line.sort_by(|a, b| {
                a.midx
                    .partial_cmp(&b.midx)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            line.into_iter()
                .map(|o| o.text)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(text: &str, midx: f64, midy: f64, height: f64) -> TextObservation {
        TextObservation {
            text: text.to_string(),
            midx,
            midy,
            height,
        }
    }

    #[test]
    fn empty_input_returns_empty_string() {
        assert_eq!(stitch_observations(Vec::new()), "");
    }

    #[test]
    fn single_observation_returns_its_text() {
        let out = stitch_observations(vec![obs("hello", 0.5, 0.5, 0.04)]);
        assert_eq!(out, "hello");
    }

    #[test]
    fn sorts_top_to_bottom() {
        let out = stitch_observations(vec![
            obs("bottom", 0.5, 0.1, 0.04),
            obs("top", 0.5, 0.9, 0.04),
            obs("middle", 0.5, 0.5, 0.04),
        ]);
        assert_eq!(out, "top\nmiddle\nbottom");
    }

    #[test]
    fn sorts_left_to_right_within_a_line() {
        let out = stitch_observations(vec![
            obs("right", 0.8, 0.5, 0.04),
            obs("left", 0.1, 0.5, 0.04),
            obs("middle", 0.5, 0.5, 0.04),
        ]);
        assert_eq!(out, "left middle right");
    }

    #[test]
    fn buckets_observations_into_lines_by_midy() {
        let out = stitch_observations(vec![
            obs("hello", 0.2, 0.9, 0.04),
            obs("world", 0.5, 0.9, 0.04),
            obs("foo", 0.2, 0.5, 0.04),
            obs("bar", 0.5, 0.5, 0.04),
        ]);
        assert_eq!(out, "hello world\nfoo bar");
    }

    #[test]
    fn near_y_observations_collapse_into_one_line() {
        let out = stitch_observations(vec![
            obs("alpha", 0.2, 0.500, 0.04),
            obs("beta", 0.5, 0.505, 0.04),
        ]);
        assert_eq!(out, "alpha beta");
    }
}
