use super::types::DocumentError;
use regex::Regex;
use uuid::Uuid;
use zeroize::Zeroize;

#[derive(Clone)]
pub struct ProtectionMap {
    pub tokenized: String,
    entries: Vec<(String, String)>,
}

impl Drop for ProtectionMap {
    fn drop(&mut self) {
        self.tokenized.zeroize();
        for (token, original) in &mut self.entries {
            token.zeroize();
            original.zeroize();
        }
    }
}

impl ProtectionMap {
    pub fn apply(text: &str, protected_terms: &[String], job: Uuid) -> Result<Self, DocumentError> {
        let pattern = Regex::new(r#"(?x)(https?://[^\s<>]+|[\w.+-]+@[\w.-]+\.[A-Za-z]{2,}|\{[^{}\r\n]{1,100}\}|\$\{[^{}\r\n]{1,100}\}|`[^`\r\n]+`|(?:[A-Za-z]:\\|/)[^\s<>\"']+)"#).map_err(|_| DocumentError::ValidationFailed)?;
        let mut ranges = pattern
            .find_iter(text)
            .map(|m| (m.start(), m.end()))
            .collect::<Vec<_>>();
        for term in protected_terms.iter().filter(|v| !v.is_empty()) {
            for (start, _) in text.match_indices(term) {
                ranges.push((start, start + term.len()));
            }
        }
        if text.trim_start().starts_with('=') {
            ranges.push((0, text.len()));
        }
        ranges.sort_unstable();
        ranges.dedup();
        let mut chosen: Vec<(usize, usize)> = Vec::new();
        for range in ranges {
            if chosen.last().is_none_or(|last| range.0 >= last.1) {
                chosen.push(range);
            }
        }
        let mut out = String::with_capacity(text.len());
        let mut entries = Vec::new();
        let mut cursor = 0;
        for (index, (start, end)) in chosen.into_iter().enumerate() {
            if !text.is_char_boundary(start) || !text.is_char_boundary(end) {
                continue;
            }
            out.push_str(&text[cursor..start]);
            let token = format!("SCX{}X{index}X", job.simple());
            out.push_str(&token);
            entries.push((token, text[start..end].to_owned()));
            cursor = end;
        }
        out.push_str(&text[cursor..]);
        Ok(Self {
            tokenized: out,
            entries,
        })
    }
    pub fn restore(&self, translated: &str) -> Result<String, DocumentError> {
        let mut result = translated.to_owned();
        for (token, original) in &self.entries {
            if result.matches(token).count() != 1 {
                return Err(DocumentError::ValidationFailed);
            }
            result = result.replacen(token, original, 1);
        }
        Ok(result)
    }
}
