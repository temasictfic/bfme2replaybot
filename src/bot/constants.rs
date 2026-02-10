/// Max replays rendered per Discord message
pub const BATCH_SIZE: usize = 10;

/// Bot's chosen max attachments per message (policy cap, not Discord's hard limit)
pub const BOT_MAX_ATTACHMENTS: usize = 10;

// Compile-time guarantee: we never try to attach more than Discord allows
const _: () = assert!(BATCH_SIZE <= BOT_MAX_ATTACHMENTS);

/// Max pending pagination entries across all channels
pub const MAX_PENDING_ENTRIES: usize = 50;

/// Per-channel cooldown in seconds
pub const COOLDOWN_SECS: u64 = 2;

/// Pending entry expiry in seconds
pub const PENDING_EXPIRY_SECS: u64 = 900;

/// Safe content limit (room for truncation suffix, under Discord's 2000 char limit)
pub const CONTENT_SAFE_LIMIT: usize = 1900;

/// Build message content from parts, truncating to stay under Discord's char limit.
/// Computes suffix only at truncation time (no per-iteration allocation).
pub fn build_safe_content(parts: &[String]) -> String {
    let mut result = String::new();
    let mut current_chars: usize = 0;

    for (i, part) in parts.iter().enumerate() {
        let part_chars = part.chars().count();
        let newline_cost = if result.is_empty() { 0 } else { 1 };
        let needed = newline_cost + part_chars;

        if current_chars + needed > CONTENT_SAFE_LIMIT {
            if i == 0 {
                // First part alone exceeds limit -- truncate it to fit
                let truncated: String = part.chars().take(CONTENT_SAFE_LIMIT).collect();
                return truncated;
            }
            // Truncate here. Compute suffix with exact skip count.
            let skipped = parts.len() - i;
            let suffix = format!("\n(+{} more...)", skipped);
            // Only append suffix if it still fits within safe limit
            if current_chars + suffix.chars().count() <= CONTENT_SAFE_LIMIT {
                result.push_str(&suffix);
            }
            return result;
        }

        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str(part);
        current_chars += needed;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_safe_content_joins_parts() {
        let parts = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(build_safe_content(&parts), "a\nb\nc");
    }

    #[test]
    fn build_safe_content_empty() {
        let parts: Vec<String> = vec![];
        assert_eq!(build_safe_content(&parts), "");
    }

    #[test]
    fn build_safe_content_truncates_long_first_part() {
        let long = "x".repeat(CONTENT_SAFE_LIMIT + 100);
        let parts = vec![long];
        let result = build_safe_content(&parts);
        assert_eq!(result.chars().count(), CONTENT_SAFE_LIMIT);
    }

    #[test]
    fn build_safe_content_truncates_with_suffix() {
        // Each part is 100 chars. CONTENT_SAFE_LIMIT is 1900, so 19 parts fit (19*100 + 18 newlines = 1918 < 1900? No.)
        // Actually: 18 parts = 18*100 + 17 newlines = 1817. 19th would need 1+100=101 more = 1918 > 1900.
        // So 18 parts fit, then truncation happens with 2 skipped.
        let parts: Vec<String> = (0..20).map(|_| "x".repeat(100)).collect();
        let result = build_safe_content(&parts);
        assert!(result.chars().count() <= CONTENT_SAFE_LIMIT);
        assert!(result.contains("(+"));
        assert!(result.contains("more...)"));
    }
}
