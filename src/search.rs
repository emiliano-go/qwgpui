use crate::model::ItemMeta;

/// Fuzzy subsequence score. Higher is better; `None` means no match.
///
/// `needle` must be lowercase; `haystack` is lowercased internally.
pub fn fuzzy_score(needle: &str, haystack: &str) -> Option<i32> {
    if needle.is_empty() {
        return Some(0);
    }
    let haystack = haystack.to_lowercase();
    let mut score = 0;
    let mut last = None;
    let mut haystack_iter = haystack.char_indices();

    for wanted in needle.chars() {
        let mut found = None;
        for (index, ch) in haystack_iter.by_ref() {
            if ch == wanted {
                found = Some(index);
                break;
            }
        }
        let index = found?;
        score += 10;
        if let Some(previous) = last {
            if index == previous + 1 {
                score += 15;
            }
        }
        if index == 0 || haystack[..index].ends_with([' ', '-', '_', '.', '/', '@']) {
            score += 20;
        }
        last = Some(index);
    }

    // Prefer shorter haystacks for the same match quality.
    Some(score - (haystack.chars().count() as i32 / 4))
}

/// Score an item against a query. Name matches dominate, then username, then URLs.
pub fn item_score(query: &str, item: &ItemMeta) -> Option<i32> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Some(0);
    }
    let mut best = fuzzy_score(&query, &item.name).map(|s| s + 400);
    for candidate in item.username.iter().chain(item.folder.iter()) {
        best = best.max(fuzzy_score(&query, candidate).map(|s| s + 200));
    }
    for uri in &item.uris {
        best = best.max(fuzzy_score(&query, uri).map(|s| s + 100));
        best = best.max(fuzzy_score(&query, &crate::model::domain_of(uri)).map(|s| s + 120));
    }
    best = best.max(fuzzy_score(&query, item.item_type.label()).map(|s| s + 40));
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ItemType;

    fn item(name: &str, user: &str, uri: &str) -> ItemMeta {
        ItemMeta {
            id: "id".into(),
            name: name.into(),
            username: Some(user.into()),
            uris: vec![uri.into()],
            folder: None,
            item_type: ItemType::Login,
        }
    }

    #[test]
    fn matches_subsequence() {
        assert!(fuzzy_score("ghb", "github").is_some());
        assert!(fuzzy_score("zzz", "github").is_none());
        assert_eq!(fuzzy_score("", "anything"), Some(0));
    }

    #[test]
    fn ranks_name_above_uri() {
        let by_name = item("github", "someone", "https://other.com");
        let by_uri = item("other", "someone", "https://github.com");
        let query = "github";
        assert!(item_score(query, &by_name) > item_score(query, &by_uri));
    }

    #[test]
    fn filters_out_non_matches() {
        assert!(item_score("nomatch", &item("github", "u", "https://github.com")).is_none());
    }
}
