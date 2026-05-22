use std::collections::BTreeSet;
use std::path::Path;

use crate::types::NodeKind;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedQuery {
    pub text: String,
    pub kinds: Vec<String>,
    pub languages: Vec<String>,
    pub path_filters: Vec<String>,
    pub name_filters: Vec<String>,
}

pub fn parse_query(raw: &str) -> ParsedQuery {
    let mut out = ParsedQuery::default();
    let tokens = tokenize_query(raw);
    let mut text_parts = Vec::new();

    for token in tokens {
        let Some(colon) = token.find(':') else {
            text_parts.push(token);
            continue;
        };
        if colon == 0 || colon == token.len() - 1 {
            text_parts.push(token);
            continue;
        }

        let key = token[..colon].to_lowercase();
        let value = unquote(&token[colon + 1..]);
        if value.is_empty() {
            text_parts.push(token);
            continue;
        }

        match key.as_str() {
            "kind" if is_valid_kind(&value) => out.kinds.push(value),
            "lang" | "language" if is_valid_language(&value) => {
                out.languages.push(value.to_lowercase())
            }
            "path" => out.path_filters.push(value),
            "name" => out.name_filters.push(value),
            _ => text_parts.push(token),
        }
    }

    out.text = text_parts.join(" ").trim().to_string();
    out
}

pub fn bounded_edit_distance(a: &str, b: &str, max_dist: usize) -> usize {
    if a == b {
        return 0;
    }
    let al = a.len();
    let bl = b.len();
    if al.abs_diff(bl) > max_dist {
        return max_dist + 1;
    }
    if al == 0 {
        return bl;
    }
    if bl == 0 {
        return al;
    }

    let a_bytes = a.as_bytes();
    let b_bytes = b.as_bytes();
    let mut prev: Vec<usize> = (0..=bl).collect();
    let mut cur = vec![0; bl + 1];

    for i in 1..=al {
        cur[0] = i;
        let mut row_min = cur[0];
        for j in 1..=bl {
            let cost = if a_bytes[i - 1] == b_bytes[j - 1] { 0 } else { 1 };
            let insertion = cur[j - 1] + 1;
            let deletion = prev[j] + 1;
            let substitution = prev[j - 1] + cost;
            cur[j] = insertion.min(deletion).min(substitution);
            row_min = row_min.min(cur[j]);
        }
        if row_min > max_dist {
            return max_dist + 1;
        }
        std::mem::swap(&mut prev, &mut cur);
    }

    prev[bl]
}

pub fn extract_search_terms(query: &str, stems: bool) -> Vec<String> {
    let mut tokens = BTreeSet::new();
    collect_compound_identifiers(query, &mut tokens);

    let camel_split = split_camel_case(query);
    let normalized = camel_split.replace(['_', '.'], " ");
    for word in normalized
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
    {
        let lower = word.to_lowercase();
        if lower.len() < 3 || is_stop_word(&lower) {
            continue;
        }
        tokens.insert(lower);
    }

    if stems {
        let existing: Vec<String> = tokens.iter().cloned().collect();
        for token in existing {
            for variant in get_stem_variants(&token) {
                if !is_stop_word(&variant) {
                    tokens.insert(variant);
                }
            }
        }
    }

    tokens.into_iter().collect()
}

pub fn score_path_relevance(file_path: &str, query: &str) -> i64 {
    let terms = extract_search_terms(query, false);
    if terms.is_empty() {
        return 0;
    }

    let path_lower = file_path.to_lowercase();
    let file_name = Path::new(file_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_lowercase();
    let dir_name = Path::new(file_path)
        .parent()
        .and_then(|parent| parent.to_str())
        .unwrap_or_default()
        .to_lowercase();

    let mut score = 0;
    for term in terms {
        if file_name.contains(&term) {
            score += 10;
        }
        if dir_name.contains(&term) {
            score += 5;
        } else if path_lower.contains(&term) {
            score += 3;
        }
    }

    let query_lower = query.to_lowercase();
    let is_test_query = query_lower.contains("test") || query_lower.contains("spec");
    if !is_test_query && is_test_file(file_path) {
        score -= 15;
    }

    score
}

pub fn name_match_bonus(node_name: &str, query: &str) -> i64 {
    let name_lower = node_name.to_lowercase();
    let raw_terms: Vec<String> = split_camel_case(query)
        .split(|c: char| c.is_whitespace() || c == '_' || c == '.' || c == '-')
        .map(|t| t.to_lowercase())
        .filter(|t| t.len() >= 2)
        .collect();
    let query_tokens: Vec<String> = query
        .split_whitespace()
        .map(|t| t.to_lowercase())
        .filter(|t| t.len() >= 2)
        .collect();
    let query_lower = query.split_whitespace().collect::<String>().to_lowercase();

    if name_lower == query_lower {
        return 80;
    }
    if query_tokens.len() > 1 && query_tokens.iter().any(|token| token == &name_lower) {
        return 60;
    }
    if name_lower.starts_with(&query_lower) && !query_lower.is_empty() {
        let ratio = query_lower.len() as f64 / name_lower.len().max(1) as f64;
        return (10.0 + 30.0 * ratio).round() as i64;
    }
    if raw_terms.len() > 1 && raw_terms.iter().all(|term| name_lower.contains(term)) {
        return 15;
    }
    if !query_lower.is_empty() && name_lower.contains(&query_lower) {
        return 10;
    }
    0
}

pub fn kind_bonus(kind: &NodeKind) -> i64 {
    match kind {
        NodeKind::Function | NodeKind::Method => 10,
        NodeKind::Interface | NodeKind::Trait | NodeKind::Protocol | NodeKind::Route => 9,
        NodeKind::Class | NodeKind::Component => 8,
        NodeKind::TypeAlias | NodeKind::Struct => 6,
        NodeKind::Enum => 5,
        NodeKind::Module | NodeKind::Namespace => 4,
        NodeKind::Property | NodeKind::Field | NodeKind::Constant | NodeKind::EnumMember => 3,
        NodeKind::Variable => 2,
        NodeKind::Import | NodeKind::Export => 1,
        NodeKind::Parameter | NodeKind::File => 0,
    }
}

pub fn is_test_file(file_path: &str) -> bool {
    let lower = file_path.to_lowercase();
    let file_name = Path::new(file_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_string();
    let lower_name = file_name.to_lowercase();

    if lower_name.starts_with("test_")
        || lower_name.starts_with("test.")
        || regex_like_separator_test_name(&lower_name)
        || regex_like_camel_test_name(&file_name)
    {
        return true;
    }

    if lower.contains("/tests/")
        || lower.contains("/test/")
        || lower.contains("/__tests__/")
        || lower.contains("/spec/")
        || lower.contains("/specs/")
        || lower.contains("/testlib/")
        || lower.contains("/testing/")
        || lower.starts_with("test/")
        || lower.starts_with("tests/")
        || lower.starts_with("spec/")
        || lower.starts_with("specs/")
        || regex_like_test_dir(file_path)
    {
        return true;
    }

    matches_non_production_dir(&lower)
}

fn tokenize_query(raw: &str) -> Vec<String> {
    let chars: Vec<char> = raw.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= chars.len() {
            break;
        }
        let start = i;
        while i < chars.len() && !chars[i].is_whitespace() {
            if chars[i] == '"' {
                if let Some(end) = chars[i + 1..].iter().position(|c| *c == '"') {
                    i += end + 2;
                    continue;
                }
                i = chars.len();
                break;
            }
            i += 1;
        }
        tokens.push(chars[start..i].iter().collect());
    }
    tokens
}

fn unquote(value: &str) -> String {
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        value[1..value.len() - 1].to_string()
    } else {
        value.to_string()
    }
}

fn is_valid_kind(value: &str) -> bool {
    matches!(
        value,
        "file"
            | "module"
            | "class"
            | "struct"
            | "interface"
            | "trait"
            | "protocol"
            | "function"
            | "method"
            | "property"
            | "field"
            | "variable"
            | "constant"
            | "enum"
            | "enum_member"
            | "type_alias"
            | "namespace"
            | "parameter"
            | "import"
            | "export"
            | "route"
            | "component"
    )
}

fn is_valid_language(value: &str) -> bool {
    matches!(
        value.to_lowercase().as_str(),
        "typescript"
            | "javascript"
            | "tsx"
            | "jsx"
            | "python"
            | "go"
            | "rust"
            | "java"
            | "c"
            | "cpp"
            | "csharp"
            | "php"
            | "ruby"
            | "swift"
            | "kotlin"
            | "dart"
            | "svelte"
            | "vue"
            | "liquid"
            | "pascal"
            | "scala"
            | "lua"
            | "luau"
            | "unknown"
    )
}

fn collect_compound_identifiers(query: &str, out: &mut BTreeSet<String>) {
    for word in query
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .filter(|w| !w.is_empty())
    {
        if word.len() < 3 {
            continue;
        }
        if looks_like_camel_compound(word) || word.contains('_') {
            out.insert(word.to_lowercase());
        }
    }
}

fn looks_like_camel_compound(word: &str) -> bool {
    let has_upper = word.chars().any(|c| c.is_ascii_uppercase());
    let has_lower = word.chars().any(|c| c.is_ascii_lowercase());
    has_upper && has_lower
}

fn split_camel_case(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 8);
    let chars: Vec<char> = input.chars().collect();
    for i in 0..chars.len() {
        let c = chars[i];
        if i > 0 {
            let prev = chars[i - 1];
            let next = chars.get(i + 1).copied();
            if (prev.is_ascii_lowercase() && c.is_ascii_uppercase())
                || (prev.is_ascii_uppercase()
                    && c.is_ascii_uppercase()
                    && next.is_some_and(|n| n.is_ascii_lowercase()))
            {
                out.push(' ');
            }
        }
        out.push(c);
    }
    out
}

fn get_stem_variants(term: &str) -> Vec<String> {
    let mut variants = BTreeSet::new();
    let t = term.to_lowercase();

    if t.ends_with("ing") && t.len() > 5 {
        let base = &t[..t.len() - 3];
        variants.insert(base.to_string());
        variants.insert(format!("{base}e"));
        let chars: Vec<char> = base.chars().collect();
        if chars.len() >= 2 && chars[chars.len() - 1] == chars[chars.len() - 2] {
            variants.insert(chars[..chars.len() - 1].iter().collect());
        }
    }
    if (t.ends_with("tion") || t.ends_with("sion")) && t.len() > 5 {
        variants.insert(t[..t.len() - 3].to_string());
    }
    if t.ends_with("ment") && t.len() > 6 {
        variants.insert(t[..t.len() - 4].to_string());
    }
    if t.ends_with("ies") && t.len() > 4 {
        variants.insert(format!("{}y", &t[..t.len() - 3]));
    } else if t.ends_with("es") && t.len() > 4 {
        variants.insert(t[..t.len() - 2].to_string());
    } else if t.ends_with('s') && !t.ends_with("ss") && t.len() > 4 {
        variants.insert(t[..t.len() - 1].to_string());
    }
    if t.ends_with("ed") && !t.ends_with("eed") && t.len() > 4 {
        variants.insert(t[..t.len() - 1].to_string());
        variants.insert(t[..t.len() - 2].to_string());
        if t.ends_with("ied") && t.len() > 5 {
            variants.insert(format!("{}y", &t[..t.len() - 3]));
        }
    }
    if t.ends_with("er") && t.len() > 4 {
        let base = &t[..t.len() - 2];
        variants.insert(base.to_string());
        variants.insert(format!("{base}e"));
        let chars: Vec<char> = base.chars().collect();
        if chars.len() >= 2 && chars[chars.len() - 1] == chars[chars.len() - 2] {
            variants.insert(chars[..chars.len() - 1].iter().collect());
        }
    }

    variants
        .into_iter()
        .filter(|variant| variant.len() >= 3 && variant != &t)
        .collect()
}

fn is_stop_word(word: &str) -> bool {
    matches!(
        word,
        "the"
            | "a"
            | "an"
            | "and"
            | "or"
            | "but"
            | "in"
            | "on"
            | "at"
            | "to"
            | "for"
            | "of"
            | "with"
            | "by"
            | "from"
            | "is"
            | "it"
            | "that"
            | "this"
            | "are"
            | "was"
            | "be"
            | "has"
            | "had"
            | "have"
            | "do"
            | "does"
            | "did"
            | "will"
            | "would"
            | "could"
            | "should"
            | "may"
            | "might"
            | "can"
            | "shall"
            | "not"
            | "no"
            | "all"
            | "each"
            | "every"
            | "how"
            | "what"
            | "where"
            | "when"
            | "who"
            | "which"
            | "why"
            | "i"
            | "me"
            | "my"
            | "we"
            | "our"
            | "you"
            | "your"
            | "he"
            | "she"
            | "they"
            | "show"
            | "give"
            | "tell"
            | "been"
            | "done"
            | "made"
            | "used"
            | "using"
            | "work"
            | "works"
            | "found"
            | "also"
            | "into"
            | "then"
            | "than"
            | "just"
            | "more"
            | "some"
            | "such"
            | "over"
            | "only"
            | "out"
            | "its"
            | "so"
            | "up"
            | "as"
            | "if"
            | "look"
            | "need"
            | "needs"
            | "want"
            | "happen"
            | "happens"
            | "affect"
            | "affected"
            | "break"
            | "breaks"
            | "failing"
            | "implemented"
            | "implement"
            | "code"
            | "file"
            | "files"
            | "function"
            | "method"
            | "class"
            | "type"
            | "fix"
            | "bug"
            | "called"
    )
}

fn regex_like_separator_test_name(lower_name: &str) -> bool {
    [".", "_", "-"].iter().any(|sep| {
        lower_name.contains(&format!("{sep}test."))
            || lower_name.contains(&format!("{sep}tests."))
            || lower_name.contains(&format!("{sep}spec."))
            || lower_name.contains(&format!("{sep}specs."))
    })
}

fn regex_like_camel_test_name(file_name: &str) -> bool {
    [
        "Test.", "Tests.", "TestCase.", "Tester.", "Spec.", "Specs.",
    ]
    .iter()
    .any(|suffix| file_name.contains(suffix))
}

fn regex_like_test_dir(file_path: &str) -> bool {
    file_path.split('/').any(|segment| {
        segment.ends_with("Test") || segment.ends_with("Tests") || segment.ends_with("Spec")
    })
}

fn matches_non_production_dir(lower_path: &str) -> bool {
    [
        "integration",
        "sample",
        "samples",
        "example",
        "examples",
        "fixture",
        "fixtures",
        "benchmark",
        "benchmarks",
        "demo",
        "demos",
    ]
    .iter()
    .any(|dir| lower_path.contains(&format!("/{dir}/")) || lower_path.starts_with(&format!("{dir}/")))
}

#[cfg(test)]
mod tests {
    use super::{bounded_edit_distance, parse_query};

    #[test]
    fn parse_query_extracts_filters() {
        let parsed = parse_query(r#"kind:function path:"src/api auth" name:Handler foo"#);
        assert_eq!(parsed.kinds, vec!["function"]);
        assert_eq!(parsed.path_filters, vec!["src/api auth"]);
        assert_eq!(parsed.name_filters, vec!["Handler"]);
        assert_eq!(parsed.text, "foo");
    }

    #[test]
    fn parse_query_preserves_unknown_prefixes() {
        let parsed = parse_query("TODO: needs review");
        assert_eq!(parsed.text, "TODO: needs review");
        assert!(parsed.kinds.is_empty());
    }

    #[test]
    fn bounded_distance_matches_budgeted_behavior() {
        assert_eq!(bounded_edit_distance("user", "user", 2), 0);
        assert_eq!(bounded_edit_distance("user", "usar", 2), 1);
        assert_eq!(bounded_edit_distance("foo", "completely-different", 2), 3);
    }
}
