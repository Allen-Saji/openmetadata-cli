//! Fully-qualified name (FQN) parsing for OpenMetadata.
//!
//! OM FQNs are dot-separated, but any segment containing a dot must be
//! double-quoted (matches OpenMetadata's `FullyQualifiedName.split` rules).
//! Example: `service."db.with.dots".schema.table.column`.

/// Split an FQN into its segments, respecting double-quoted segments.
///
/// Quoted segments keep the surrounding quotes stripped. Inner quotes are
/// not handled (OM doesn't allow escaped quotes inside identifiers).
pub fn split(fqn: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut in_quote = false;
    for c in fqn.chars() {
        match c {
            '"' => in_quote = !in_quote,
            '.' if !in_quote => {
                out.push(std::mem::take(&mut buf));
            }
            _ => buf.push(c),
        }
    }
    out.push(buf);
    out
}

/// Re-join segments back into a quoted FQN. Segments with dots are quoted.
pub fn join(parts: &[String]) -> String {
    parts
        .iter()
        .map(|p| {
            if p.contains('.') {
                format!("\"{p}\"")
            } else {
                p.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(".")
}

/// Return the parent FQN (everything except the last segment), and the last
/// segment, or `None` if the FQN has fewer than two segments.
pub fn split_last(fqn: &str) -> Option<(String, String)> {
    let parts = split(fqn);
    if parts.len() < 2 {
        return None;
    }
    let last = parts.last().cloned()?;
    let parent = join(&parts[..parts.len() - 1]);
    Some((parent, last))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_plain() {
        assert_eq!(
            split("svc.db.schema.orders"),
            vec!["svc", "db", "schema", "orders"]
        );
    }

    #[test]
    fn splits_quoted_with_dot() {
        assert_eq!(
            split("svc.\"db.with.dots\".schema.orders"),
            vec!["svc", "db.with.dots", "schema", "orders"]
        );
    }

    #[test]
    fn rejoins_with_quotes() {
        let parts = vec![
            "svc".to_string(),
            "db.with.dots".to_string(),
            "schema".to_string(),
        ];
        assert_eq!(join(&parts), "svc.\"db.with.dots\".schema");
    }

    #[test]
    fn split_last_returns_parent_and_leaf() {
        let (parent, leaf) = split_last("svc.db.schema.orders.email").unwrap();
        assert_eq!(parent, "svc.db.schema.orders");
        assert_eq!(leaf, "email");
    }

    #[test]
    fn split_last_handles_quoted_parent() {
        let (parent, leaf) = split_last("svc.\"db.x\".schema.orders.email").unwrap();
        assert_eq!(parent, "svc.\"db.x\".schema.orders");
        assert_eq!(leaf, "email");
    }

    #[test]
    fn split_last_none_for_single_segment() {
        assert!(split_last("svc").is_none());
    }
}
