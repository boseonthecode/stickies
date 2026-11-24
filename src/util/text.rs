pub fn note_title(body: &str) -> String {
    let line = body
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");

    if line.is_empty() {
        return "Empty note".to_owned();
    }

    let mut title = line.chars().take(120).collect::<String>();
    if line.chars().count() > 120 {
        title.push('…');
    }
    title
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_titles_use_the_first_non_empty_line() {
        assert_eq!(note_title("\n  First line  \nSecond line"), "First line");
        assert_eq!(note_title("   \n\t"), "Empty note");
    }

    #[test]
    fn note_titles_truncate_long_lines() {
        let body = "x".repeat(200);

        let title = note_title(&body);

        assert_eq!(title.chars().count(), 121);
        assert!(title.ends_with('…'));
    }
}
