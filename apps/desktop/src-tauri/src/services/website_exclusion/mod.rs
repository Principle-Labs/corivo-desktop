//! Website host exclusion (companion to the bundle-id exclusion engine).
//!
//! **Scope (minimum viable):** users list host patterns ("notion.so",
//! "*.example.com") that should be excluded from context awareness. The
//! capture pipeline calls [`WebsiteExclusionEngine::is_excluded`] with
//! the URL host extracted from the active browser's AX tree; on a hit,
//! the frame's text is skipped the same way an app exclusion skips it.
//!
//! Pattern grammar (small on purpose):
//!   - Exact host:        `notion.so`     matches only `notion.so`
//!   - Wildcard suffix:   `*.notion.so`   matches `app.notion.so`,
//!                                         `foo.bar.notion.so`,
//!                                         but **not** bare `notion.so`
//!   - Bare host implicitly covers subdomains too:
//!     `notion.so` matches `notion.so` AND `app.notion.so`. The wildcard
//!     form exists for users who want to exclude subdomains only
//!     (rare but real — e.g. allow `notion.so/marketing` but exclude
//!     `app.notion.so`).
//!
//! All matching is case-insensitive (hosts are lowercased on parse). Ports
//! and userinfo are stripped before comparison; paths and query strings
//! are ignored — exclusion is a host-level concept.
//!
//! **ASCII-only.** Patterns must be ASCII (punycode form for IDN hosts).
//! A user-typed `münchen.de` would never match the browser's `xn--mnchen-3ya.de`
//! after IDN normalisation, so we reject non-ASCII at parse time and let
//! the UI surface a clear error instead of silently storing dead patterns.
//! If IDN support becomes a real ask we'll wire the `idna` crate in and
//! normalise both sides; until then, rejection is the safer default.

use std::collections::HashSet;

/// Parsed user pattern.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WebsitePattern {
    /// `notion.so` — matches the host itself and any subdomain.
    Bare(String),
    /// `*.notion.so` — matches strict subdomains only.
    WildcardSuffix(String),
}

impl WebsitePattern {
    /// Parse a user-typed pattern. Trims whitespace, lowercases, strips
    /// a leading scheme + path if the user pasted a full URL. Returns
    /// `None` if the input is empty after normalization or contains
    /// non-ASCII characters (see the module doc for why we reject IDN
    /// input rather than guess at normalisation).
    pub fn parse(input: &str) -> Option<Self> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return None;
        }
        // Tolerate users pasting full URLs.
        let without_scheme = trimmed
            .split_once("://")
            .map(|(_, rest)| rest)
            .unwrap_or(trimmed);
        // Drop path / query / fragment / userinfo / port.
        let host_only = without_scheme
            .split(|c: char| c == '/' || c == '?' || c == '#')
            .next()
            .unwrap_or(without_scheme);
        let host_only = host_only
            .rsplit_once('@')
            .map(|(_, h)| h)
            .unwrap_or(host_only);
        let host_only = host_only
            .split_once(':')
            .map(|(h, _)| h)
            .unwrap_or(host_only);
        // Reject non-ASCII (IDN) input. Browsers report hosts in punycode
        // form; storing the unicode form would silently never match.
        if !host_only.is_ascii() {
            return None;
        }
        let lower = host_only.to_ascii_lowercase();

        if let Some(rest) = lower.strip_prefix("*.") {
            if rest.is_empty() || !rest.contains('.') {
                // `*.` alone or `*.tld` is too broad to be useful and
                // smells like a typo. Bail rather than silently match
                // everything.
                return None;
            }
            Some(Self::WildcardSuffix(rest.to_string()))
        } else if lower.contains('.') {
            Some(Self::Bare(lower))
        } else {
            // Single label hosts (`localhost`) are technically valid
            // but exclusion-of-localhost is rarely what the user means.
            // Treat them as bare matches so we don't silently swallow them.
            Some(Self::Bare(lower))
        }
    }

    /// Round-trip back to the canonical string form. Stable so we can
    /// store it verbatim in `Config` and re-parse on next launch.
    pub fn display(&self) -> String {
        match self {
            Self::Bare(host) => host.clone(),
            Self::WildcardSuffix(suffix) => format!("*.{suffix}"),
        }
    }

    /// True when this pattern matches a URL's host (already lowercased,
    /// no port / userinfo). Caller is responsible for normalisation —
    /// [`normalize_host`] is the helper that ships with this module.
    pub fn matches(&self, host: &str) -> bool {
        match self {
            Self::Bare(target) => host == target || host.ends_with(&format!(".{target}")),
            Self::WildcardSuffix(suffix) => host.ends_with(&format!(".{suffix}")),
        }
    }
}

/// Lowercase a host, strip port + userinfo. Returns `None` if there's
/// no host to extract (empty input, scheme-only, etc.).
pub fn normalize_host(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let without_scheme = trimmed
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(trimmed);
    let host_only = without_scheme
        .split(|c: char| c == '/' || c == '?' || c == '#')
        .next()
        .unwrap_or(without_scheme);
    let host_only = host_only
        .rsplit_once('@')
        .map(|(_, h)| h)
        .unwrap_or(host_only);
    let host_only = host_only
        .split_once(':')
        .map(|(h, _)| h)
        .unwrap_or(host_only);
    if host_only.is_empty() {
        return None;
    }
    Some(host_only.to_ascii_lowercase())
}

/// Stateless engine. Cheap to construct from a flat list of patterns
/// stored in `Config.exclusion.extra_website_patterns`.
#[derive(Debug, Clone, Default)]
pub struct WebsiteExclusionEngine {
    patterns: Vec<WebsitePattern>,
}

impl WebsiteExclusionEngine {
    pub fn new<I: IntoIterator<Item = WebsitePattern>>(patterns: I) -> Self {
        // De-dup while preserving order — config can grow stale across
        // app versions and we don't want duplicate UI rows.
        let mut seen: HashSet<WebsitePattern> = HashSet::new();
        let mut keep = Vec::new();
        for pat in patterns {
            if seen.insert(pat.clone()) {
                keep.push(pat);
            }
        }
        Self { patterns: keep }
    }

    /// Convenience: build from the raw user-string list in `Config`.
    /// Invalid entries are dropped silently (the user can't surface
    /// them in the UI anyway because they'd never have made it through
    /// the add command's validation).
    pub fn from_strings<I: IntoIterator<Item = String>>(raw: I) -> Self {
        Self::new(raw.into_iter().filter_map(|s| WebsitePattern::parse(&s)))
    }

    /// True when `url_or_host` matches any pattern in the engine.
    /// Accepts a full URL or a bare host — both go through
    /// [`normalize_host`]. Returns `false` for unparseable input rather
    /// than erroring; "I don't know what this is" should not silently
    /// hide context.
    pub fn is_excluded(&self, url_or_host: &str) -> bool {
        let Some(host) = normalize_host(url_or_host) else {
            return false;
        };
        self.patterns.iter().any(|p| p.matches(&host))
    }

    pub fn len(&self) -> usize {
        self.patterns.len()
    }

    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bare_host() {
        let p = WebsitePattern::parse("Notion.so").unwrap();
        assert_eq!(p, WebsitePattern::Bare("notion.so".into()));
        assert_eq!(p.display(), "notion.so");
    }

    #[test]
    fn parses_wildcard_suffix() {
        let p = WebsitePattern::parse("*.example.com").unwrap();
        assert_eq!(p, WebsitePattern::WildcardSuffix("example.com".into()));
        assert_eq!(p.display(), "*.example.com");
    }

    #[test]
    fn rejects_bare_wildcard_or_tld_only() {
        assert!(WebsitePattern::parse("*.").is_none());
        assert!(WebsitePattern::parse("*.com").is_none());
    }

    #[test]
    fn strips_scheme_path_port_userinfo() {
        let p = WebsitePattern::parse("https://user:pw@app.notion.so:443/foo?bar#baz").unwrap();
        assert_eq!(p, WebsitePattern::Bare("app.notion.so".into()));
    }

    #[test]
    fn empty_input_returns_none() {
        assert!(WebsitePattern::parse("").is_none());
        assert!(WebsitePattern::parse("   ").is_none());
    }

    #[test]
    fn bare_pattern_matches_subdomains_too() {
        let p = WebsitePattern::parse("notion.so").unwrap();
        assert!(p.matches("notion.so"));
        assert!(p.matches("app.notion.so"));
        assert!(p.matches("foo.bar.notion.so"));
        assert!(!p.matches("notion.so.evil.com"));
        assert!(!p.matches("notnotion.so"));
    }

    #[test]
    fn wildcard_pattern_excludes_bare_host() {
        let p = WebsitePattern::parse("*.notion.so").unwrap();
        assert!(p.matches("app.notion.so"));
        assert!(p.matches("foo.bar.notion.so"));
        assert!(!p.matches("notion.so"), "wildcard form is subdomains-only");
    }

    #[test]
    fn engine_matches_against_full_url() {
        let engine = WebsiteExclusionEngine::from_strings(["app.notion.so".to_string()]);
        assert!(engine.is_excluded("https://app.notion.so/page-1"));
        assert!(engine.is_excluded("APP.NOTION.SO"));
        assert!(!engine.is_excluded("notion.com"));
    }

    #[test]
    fn engine_dedupes_equal_patterns() {
        let engine = WebsiteExclusionEngine::from_strings(vec![
            "notion.so".to_string(),
            "  NOTION.SO  ".to_string(), // normalises to same thing
            "*.notion.so".to_string(),
        ]);
        assert_eq!(engine.len(), 2);
    }

    #[test]
    fn engine_ignores_unparseable_input() {
        let engine = WebsiteExclusionEngine::from_strings(vec!["".to_string(), "*.".to_string()]);
        assert!(engine.is_empty());
        assert!(!engine.is_excluded("anything.example.com"));
    }

    #[test]
    fn normalize_host_drops_port_and_userinfo() {
        assert_eq!(
            normalize_host("https://user:pw@app.notion.so:443/foo"),
            Some("app.notion.so".to_string())
        );
        assert_eq!(normalize_host(""), None);
        assert_eq!(normalize_host("   "), None);
    }

    #[test]
    fn rejects_non_ascii_idn_input() {
        // Raw unicode would never match the browser's punycode-encoded
        // host. Reject at parse time rather than storing dead patterns.
        assert!(WebsitePattern::parse("münchen.de").is_none());
        assert!(WebsitePattern::parse("例子.cn").is_none());
        assert!(WebsitePattern::parse("*.例子.cn").is_none());
        // Mixed: ASCII subdomain on a unicode root is still non-ASCII overall.
        assert!(WebsitePattern::parse("app.münchen.de").is_none());
    }

    #[test]
    fn accepts_punycode_form_for_idn() {
        // The escape hatch: users (or future code) who want to exclude an
        // IDN host can supply the already-encoded form.
        let p = WebsitePattern::parse("xn--mnchen-3ya.de").unwrap();
        assert_eq!(p, WebsitePattern::Bare("xn--mnchen-3ya.de".into()));
        assert!(p.matches("xn--mnchen-3ya.de"));
    }
}
