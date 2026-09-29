use nucleo::pattern::{CaseMatching, Normalization, Pattern};
use nucleo::{Config, Matcher};

/// Fuzzy-filters `items` by `query`, best match first. An empty query keeps everything.
pub fn fuzzy_filter(query: &str, items: &[String]) -> Vec<String> {
    let mut matcher = Matcher::new(Config::DEFAULT);
    Pattern::parse(query, CaseMatching::Smart, Normalization::Smart)
        .match_list(items.iter().map(String::as_str), &mut matcher)
        .into_iter()
        .map(|(item, _score)| item.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<String> {
        ["chapter_01.md", "chapter_02.md", "notes.txt", "outline.md"]
            .map(String::from)
            .to_vec()
    }

    #[test]
    fn matches_subsequences() {
        let found = fuzzy_filter("ch2", &items());
        assert_eq!(found, vec!["chapter_02.md".to_string()]);
    }

    #[test]
    fn non_matching_query_returns_nothing() {
        assert!(fuzzy_filter("zzz", &items()).is_empty());
    }

    #[test]
    fn empty_query_keeps_everything() {
        assert_eq!(fuzzy_filter("", &items()).len(), 4);
    }
}
