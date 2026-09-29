use std::ops::Range;

/// Byte ranges of every non-overlapping, case-sensitive occurrence of `query` in `text`.
/// An empty query matches nothing.
pub fn find_all(text: &str, query: &str) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    text.match_indices(query)
        .map(|(start, m)| start..start + m.len())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_match_and_empty_query_return_nothing() {
        assert!(find_all("It was a dark night", "storm").is_empty());
        assert!(find_all("It was a dark night", "").is_empty());
        assert!(find_all("", "a").is_empty());
    }

    #[test]
    fn finds_every_match_as_byte_ranges() {
        assert_eq!(find_all("été, été", "été"), [0..5, 7..12]);
        // Matches never overlap.
        assert_eq!(find_all("aaaaa", "aa"), [0..2, 2..4]);
    }
}
