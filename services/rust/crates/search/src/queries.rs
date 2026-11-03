//! Deterministic query generation and relevance scoring for one tender
//! position. The same inputs always produce the same query list — runs stay
//! reproducible and auditable, with no randomness and no hidden state.

use serde_json::Value as Json;

/// One generated query and its `search_queries.kind` value
/// (CHECK: exact | alt | manufacturer | model | specs | synonym |
/// multilingual | custom).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryCandidate {
    pub query: String,
    pub kind: String,
}

/// Components row slice needed for generation.
pub struct QueryInput<'a> {
    pub name: &'a str,
    pub category: &'a str,
    /// `components.search_hints` JSONB.
    pub hints: &'a Json,
    /// (name, value_text, unit, data_type) for each requirement.
    pub requirements: &'a [(String, String, String, String)],
}

/// Small deterministic RU↔EN dictionary for technical nouns. Only whole
/// names whose significant tokens are all covered are translated — a
/// partial machine translation would silently change the meaning, so we
/// skip instead of guessing.
const RU_TO_EN: &[(&str, &str)] = &[
    ("насос", "pump"),
    ("центробежный", "centrifugal"),
    ("насосная", "pump"),
    ("станция", "station"),
    ("клапан", "valve"),
    ("задвижка", "gate valve"),
    ("кран", "cock valve"),
    ("фильтр", "filter"),
    ("фильтрующий", "filter"),
    ("кабель", "cable"),
    ("провод", "wire"),
    ("проводник", "wire"),
    ("труба", "pipe"),
    ("трубопровод", "pipeline"),
    ("компрессор", "compressor"),
    ("двигатель", "motor"),
    ("редуктор", "gearbox"),
    ("подшипник", "bearing"),
    ("герметик", "sealant"),
    ("утеплитель", "insulation"),
    ("изоляция", "insulation"),
    ("лампа", "lamp"),
    ("светильник", "luminaire"),
    ("выключатель", "switch"),
    ("розетка", "socket"),
    ("breaker", "breaker"),
    ("автомат", "circuit breaker"),
    ("датчик", "sensor"),
    ("расходомер", "flow meter"),
    ("счетчик", "meter"),
    ("манометр", "pressure gauge"),
    ("термометр", "thermometer"),
    ("винт", "bolt"),
    ("болт", "bolt"),
    ("гайка", "nut"),
    ("крепеж", "fastener"),
    ("кронштейн", "bracket"),
    ("насосов", "pumps"),
    ("клапанов", "valves"),
    ("фильтров", "filters"),
    ("кабелей", "cables"),
    ("труб", "pipes"),
];

const STOPWORDS: &[&str] = &[
    "the", "and", "for", "with", "from", "that", "this", "are", "was", "were", "has", "have",
    "not", "but", "you", "all", "can", "her", "his", "its", "our", "out", "who", "get", "how", "и",
    "в", "на", "для", "из", "по", "с", "к", "от", "до", "что", "как", "не", "при", "за", "или",
    "а", "но", "это", "же", "бы", "вы", "он", "она", "они", "мы", "их", "ее", "ему",
];

/// Unicode Cyrillic block U+0400..U+04FF (Russian, Ukrainian, etc.).
fn is_cyrillic(ch: char) -> bool {
    ('\u{0400}'..='\u{04FF}').contains(&ch)
}

fn normalize(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|ch: char| !ch.is_alphanumeric() && ch.is_ascii() || !ch.is_alphanumeric())
        .map(str::trim)
        .filter(|token| token.chars().count() > 1 && !STOPWORDS.contains(token))
        .map(str::to_string)
        .collect()
}

/// Build the ordered, deduplicated query list for one component.
/// Priority: exact → alt → manufacturer → model → specs → custom hints →
/// synonym → multilingual. The result is capped at `max` queries.
pub fn generate_queries(input: &QueryInput<'_>, max: usize) -> Vec<QueryCandidate> {
    let mut out: Vec<QueryCandidate> = Vec::new();
    let mut seen: Vec<String> = Vec::new();

    let push =
        |query: String, kind: &str, out: &mut Vec<QueryCandidate>, seen: &mut Vec<String>| {
            let query = normalize(&query);
            if query.is_empty() || seen.contains(&query) || out.len() >= max {
                return;
            }
            seen.push(query.clone());
            out.push(QueryCandidate {
                query,
                kind: kind.to_string(),
            });
        };

    let name = input.name.trim();
    if !name.is_empty() {
        push(name.to_string(), "exact", &mut out, &mut seen);
    }

    if let Some(alternates) = input.hints.get("alternate_names").and_then(Json::as_array) {
        for alt in alternates.iter().filter_map(Json::as_str).take(max) {
            push(alt.to_string(), "alt", &mut out, &mut seen);
        }
    }
    if let Some(english) = input.hints.get("english_name").and_then(Json::as_str) {
        if !english.is_empty() {
            push(english.to_string(), "alt", &mut out, &mut seen);
        }
    }

    let manufacturer = input
        .hints
        .get("manufacturer")
        .and_then(Json::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if !manufacturer.is_empty() {
        push(
            format!("{manufacturer} {name}"),
            "manufacturer",
            &mut out,
            &mut seen,
        );
        let model = input
            .hints
            .get("model")
            .and_then(Json::as_str)
            .unwrap_or("");
        if !model.is_empty() {
            push(
                format!("{manufacturer} {model}"),
                "model",
                &mut out,
                &mut seen,
            );
        }
    } else if let Some(model) = input.hints.get("model").and_then(Json::as_str) {
        if !model.is_empty() && !model.eq_ignore_ascii_case(name) {
            push(model.to_string(), "model", &mut out, &mut seen);
        }
    }

    // Spec-bearing requirements make the strongest additional queries:
    // standards, materials, models and numbered dimensions.
    for (req_name, value, unit, data_type) in input.requirements {
        if out.len() >= max {
            break;
        }
        let interesting = matches!(
            data_type.as_str(),
            "STANDARD" | "MATERIAL" | "MODEL" | "DIMENSION"
        ) || (data_type == "NUMBER" && !value.is_empty());
        if !interesting || value.trim().is_empty() {
            continue;
        }
        let suffix = if unit.trim().is_empty() {
            value.clone()
        } else {
            format!("{value} {}", unit.trim())
        };
        push(format!("{req_name} {suffix}"), "specs", &mut out, &mut seen);
    }

    if let Some(suggested) = input
        .hints
        .get("suggested_queries")
        .and_then(Json::as_array)
    {
        for query in suggested.iter().filter_map(Json::as_str).take(max) {
            push(query.to_string(), "custom", &mut out, &mut seen);
        }
    }

    // Deterministic dictionary translation of the whole name.
    if let Some(translated) = translate_name(name) {
        push(translated, "synonym", &mut out, &mut seen);
    }

    // Multilingual context: Cyrillic names get English procurement context,
    // Latin names get Russian context — both are plain suffixes.
    if !name.is_empty() {
        if name.chars().any(is_cyrillic) {
            push(
                format!("{name} specification datasheet"),
                "multilingual",
                &mut out,
                &mut seen,
            );
        } else {
            push(
                format!("{name} характеристики"),
                "multilingual",
                &mut out,
                &mut seen,
            );
        }
    }

    // Category as a last resort when it adds information.
    if out.len() < max {
        let category = input.category.trim();
        if !category.is_empty() && !category.eq_ignore_ascii_case(name) {
            push(category.to_string(), "custom", &mut out, &mut seen);
        }
    }

    out.truncate(max);
    out
}

/// Translate a Russian component name to English via the dictionary.
/// Returns `None` unless every significant token is covered — partial
/// translations would silently distort the query.
fn translate_name(name: &str) -> Option<String> {
    let tokens = tokenize(name);
    if tokens.is_empty() {
        return None;
    }
    let mut translated = Vec::with_capacity(tokens.len());
    for token in &tokens {
        match RU_TO_EN
            .iter()
            .find(|(ru, _)| ru == token)
            .map(|(_, en)| (*en).to_string())
        {
            Some(en) => translated.push(en),
            // Numbers and latin technical tokens pass through unchanged.
            None if token.chars().all(|ch| ch.is_ascii_digit()) => translated.push(token.clone()),
            None if token.chars().all(|ch| !is_cyrillic(ch)) => translated.push(token.clone()),
            None => return None,
        }
    }
    let result = translated.join(" ");
    if result.eq_ignore_ascii_case(name) {
        None
    } else {
        Some(result)
    }
}

/// Deterministic relevance score in `0..1`: term coverage of the query in
/// title+snippet, weighted toward the title, plus a bonus when the whole
/// normalized query appears verbatim. Empty inputs score 0.
pub fn relevance_score(query: &str, title: &str, snippet: &str) -> f64 {
    let terms = tokenize(query);
    if terms.is_empty() {
        return 0.0;
    }
    let title_lower = title.to_lowercase();
    let text_lower = format!("{title_lower} {}", snippet.to_lowercase());

    let total = terms.len() as f64;
    let in_text = terms
        .iter()
        .filter(|term| text_lower.contains(term.as_str()))
        .count() as f64;
    let in_title = terms
        .iter()
        .filter(|term| title_lower.contains(term.as_str()))
        .count() as f64;

    let mut score = 0.7 * (in_text / total) + 0.3 * (in_title / total);
    let normalized_query = normalize(query);
    if !normalized_query.is_empty() && text_lower.contains(&normalized_query) {
        score += 0.2;
    }
    score.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn input<'a>(
        name: &'a str,
        hints: &'a Json,
        reqs: &'a [(String, String, String, String)],
    ) -> QueryInput<'a> {
        QueryInput {
            name,
            category: "",
            hints,
            requirements: reqs,
        }
    }

    #[test]
    fn exact_query_always_first_and_deduplicated() {
        let hints = json!({"alternate_names": ["Насос сетевой"]});
        let queries = generate_queries(&input("Насос центробежный", &hints, &[]), 12);
        assert_eq!(queries[0].query, "насос центробежный");
        assert_eq!(queries[0].kind, "exact");
        let mut seen_set = std::collections::HashSet::new();
        for q in &queries {
            assert!(
                seen_set.insert(q.query.clone()),
                "duplicate query in {queries:?}"
            );
        }
        assert_eq!(queries[1].kind, "alt");
    }

    #[test]
    fn generation_is_deterministic() {
        let hints = json!({
            "manufacturer": "Grundfos",
            "model": "CR 10",
            "alternate_names": ["насос сетевой"],
            "suggested_queries": ["Grundfos CR 10 купить"]
        });
        let reqs = vec![(
            "Стандарт".to_string(),
            "ISO 5167".to_string(),
            String::new(),
            "STANDARD".to_string(),
        )];
        let a = generate_queries(&input("Насос", &hints, &reqs), 12);
        let b = generate_queries(&input("Насос", &hints, &reqs), 12);
        assert_eq!(a, b, "same inputs must produce the same queries");
    }

    #[test]
    fn manufacturer_model_and_specs_appear_in_priority_order() {
        let hints = json!({"manufacturer": "ABB", "model": "S200"});
        let reqs = vec![(
            "Материал".to_string(),
            "нержавеющая сталь".to_string(),
            String::new(),
            "MATERIAL".to_string(),
        )];
        let queries = generate_queries(&input("Автоматический выключатель", &hints, &reqs), 12);
        let kinds: Vec<_> = queries.iter().map(|q| q.kind.as_str()).collect();
        assert!(kinds.contains(&"manufacturer"), "{kinds:?}");
        assert!(kinds.contains(&"model"), "{kinds:?}");
        assert!(kinds.contains(&"specs"), "{kinds:?}");
        let man_pos = kinds.iter().position(|k| *k == "manufacturer").unwrap();
        let model_pos = kinds.iter().position(|k| *k == "model").unwrap();
        assert!(man_pos < model_pos);
        assert!(kinds.contains(&"exact"));
    }

    #[test]
    fn cap_is_respected() {
        let hints = json!({"alternate_names": ["a1","a2","a3","a4","a5","a6"]});
        let queries = generate_queries(&input("Насос", &hints, &[]), 3);
        assert_eq!(queries.len(), 3);
    }

    #[test]
    fn full_russian_name_translates_but_partial_does_not() {
        assert_eq!(
            translate_name("Насос центробежный").as_deref(),
            Some("pump centrifugal")
        );
        // "магнитный" is not in the dictionary → no half-translation.
        assert_eq!(translate_name("Насос магнитный"), None);
        // Pure latin passes through (no translation needed → None from eq check).
        assert_eq!(translate_name("gate valve"), None);
    }

    #[test]
    fn relevance_prefers_title_matches_and_exact_phrase() {
        let high = relevance_score(
            "насос центробежный",
            "Насос центробежный Grundfos CR 10",
            "catalogue page",
        );
        let low = relevance_score("насос центробежный", "Разное оборудование", "нет данных");
        assert!(high > 0.8, "high={high}");
        assert_eq!(low, 0.0);
        assert_eq!(relevance_score("", "x", "y"), 0.0);
    }

    #[test]
    fn relevance_is_bounded_and_stable() {
        let score = relevance_score("фильтр", "фильтр фильтр", "фильтр");
        assert!((0.0..=1.0).contains(&score));
        assert_eq!(
            score,
            relevance_score("фильтр", "фильтр фильтр", "фильтр"),
            "score must be deterministic"
        );
    }
}
