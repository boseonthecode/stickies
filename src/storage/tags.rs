pub fn normalize_tag(raw: &str) -> Option<String> {
    let tag = raw.trim().trim_start_matches('#').trim();

    (!tag.is_empty()).then(|| tag.to_owned())
}

pub fn parse_tags(input: &str) -> Vec<String> {
    let mut tags = Vec::new();

    for raw in input.split(|character: char| character == ',' || character.is_whitespace()) {
        let Some(tag) = normalize_tag(raw) else {
            continue;
        };
        if !contains_tag(&tags, &tag) {
            tags.push(tag);
        }
    }

    tags
}

pub fn normalize_tags(tags: &[String]) -> Vec<String> {
    let mut normalized = Vec::new();

    for raw in tags {
        let Some(tag) = normalize_tag(raw) else {
            continue;
        };
        if !contains_tag(&normalized, &tag) {
            normalized.push(tag);
        }
    }

    normalized
}

pub fn merge_tags(existing: &mut Vec<String>, incoming: &[String]) -> bool {
    let mut changed = false;

    for raw in incoming {
        let Some(tag) = normalize_tag(raw) else {
            continue;
        };
        if !contains_tag(existing, &tag) {
            existing.push(tag);
            changed = true;
        }
    }

    changed
}

pub fn remove_tag(existing: &mut Vec<String>, tag: &str) -> bool {
    let Some(tag) = normalize_tag(tag) else {
        return false;
    };
    let before = existing.len();
    existing.retain(|current| !same_tag(current, &tag));

    existing.len() != before
}

pub fn contains_tag(tags: &[String], tag: &str) -> bool {
    tags.iter().any(|current| same_tag(current, tag))
}

fn same_tag(left: &str, right: &str) -> bool {
    left.to_lowercase() == right.to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_hashes_and_whitespace() {
        assert_eq!(normalize_tag("  ##work  ").as_deref(), Some("work"));
        assert_eq!(normalize_tag("   #   "), None);
        assert_eq!(normalize_tag(""), None);
    }

    #[test]
    fn parses_comma_and_space_separated_tags() {
        assert_eq!(
            parse_tags("#work, idea  #Work\npersonal"),
            vec!["work", "idea", "personal"]
        );
    }

    #[test]
    fn normalizes_and_deduplicates_case_insensitively() {
        let tags = vec![
            " Work ".to_owned(),
            "work".to_owned(),
            "#IDEA".to_owned(),
            "idea".to_owned(),
            "  ".to_owned(),
        ];

        assert_eq!(normalize_tags(&tags), vec!["Work", "IDEA"]);
    }

    #[test]
    fn merges_and_removes_tags_case_insensitively() {
        let mut tags = vec!["Work".to_owned()];
        assert!(merge_tags(
            &mut tags,
            &["work".to_owned(), "idea".to_owned()]
        ));
        assert!(contains_tag(&tags, "IDEA"));
        assert!(remove_tag(&mut tags, "#work"));
        assert!(!contains_tag(&tags, "work"));
        assert!(!remove_tag(&mut tags, "missing"));
    }
}
