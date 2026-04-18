//! Entity types that support the OpenMetadata CSV export/import endpoints.

use crate::error::{CliError, CliResult};

/// Canonical entity types with CSV endpoints and their REST collection base.
const SUPPORTED: &[(&str, &str)] = &[
    ("table", "v1/tables"),
    ("database", "v1/databases"),
    ("databaseSchema", "v1/databaseSchemas"),
    ("glossary", "v1/glossaries"),
    ("glossaryTerm", "v1/glossaryTerms"),
    ("team", "v1/teams"),
    ("user", "v1/users"),
    ("databaseService", "v1/services/databaseServices"),
    ("securityService", "v1/services/securityServices"),
    ("driveService", "v1/services/driveServices"),
    ("testCase", "v1/dataQuality/testCases"),
];

/// Resolve a user-provided entity type to its REST collection base path.
///
/// Accepts kebab-case and snake_case aliases (e.g. `database-schema`,
/// `database_schema`) so the CLI feels consistent with the rest of `omd`.
pub fn collection_for(t: &str) -> CliResult<&'static str> {
    let normalized = normalize(t);
    for (canon, path) in SUPPORTED {
        if normalize(canon) == normalized {
            return Ok(path);
        }
    }
    Err(CliError::InvalidInput(format!(
        "entity type `{t}` does not support CSV. supported: {}",
        SUPPORTED
            .iter()
            .map(|(c, _)| *c)
            .collect::<Vec<_>>()
            .join(", ")
    )))
}

fn normalize(s: &str) -> String {
    s.chars()
        .filter_map(|c| {
            if c.is_ascii_alphanumeric() {
                Some(c.to_ascii_lowercase())
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_canonical_and_aliases() {
        assert_eq!(collection_for("table").unwrap(), "v1/tables");
        assert_eq!(
            collection_for("databaseSchema").unwrap(),
            "v1/databaseSchemas"
        );
        assert_eq!(
            collection_for("database-schema").unwrap(),
            "v1/databaseSchemas"
        );
        assert_eq!(
            collection_for("database_schema").unwrap(),
            "v1/databaseSchemas"
        );
        assert_eq!(collection_for("GlossaryTerm").unwrap(), "v1/glossaryTerms");
    }

    #[test]
    fn unknown_type_lists_supported() {
        let e = collection_for("pipeline").unwrap_err();
        let msg = e.to_string();
        assert!(msg.contains("does not support CSV"));
        assert!(msg.contains("table"));
    }
}
