//! Deterministic technical matcher.
//!
//! One requirement is compared against one product spec; the overall verdict
//! follows the contract in `proto/matching/v1/matching.proto`:
//!
//! * `FULL` — every evaluable requirement matches.
//! * `TOLERANCE` — numeric values deviate within the requirement tolerance.
//! * `PARTIAL` — non-mandatory checks are partial/missing; mandatory all pass.
//! * `MISMATCH` — at least one mandatory requirement fails on data.
//! * `INSUFFICIENT` — at least one mandatory requirement has no product data
//!   (never treated as a match).
//!
//! No heuristics beyond explicit normalization: units are compared textually
//! (no conversion), text comparison is normalized-exact or substring.

use crate::tender::matching::v1::{
    MatchResult, MatchVerdict, ProductCandidate, Requirement, RequirementCheck, SpecValue,
};

/// Compare floats within a relative epsilon when no tolerance is configured.
const EXACT_EPSILON: f64 = 1e-6;

/// Match every requirement of a component against every product candidate.
pub fn match_component(
    component_id: &str,
    requirements: &[Requirement],
    products: &[ProductCandidate],
) -> Vec<MatchResult> {
    products
        .iter()
        .filter(|product| !product.product_id.trim().is_empty())
        .map(|product| match_product(component_id, requirements, product))
        .collect()
}

/// Match one product against the component requirements.
pub fn match_product(
    component_id: &str,
    requirements: &[Requirement],
    product: &ProductCandidate,
) -> MatchResult {
    let mut checks: Vec<RequirementCheck> = Vec::with_capacity(requirements.len());
    let mut missing_specs: Vec<String> = Vec::new();
    let mut weighted_sum = 0.0f64;
    let mut weight_total = 0.0f64;

    for requirement in requirements {
        let weight = if requirement.mandatory { 2.0 } else { 1.0 };
        let spec = find_spec(&product.specs, &requirement.name);
        let (check, quality) = match spec {
            Some(spec) => evaluate(requirement, spec),
            None => {
                missing_specs.push(requirement.name.clone());
                (
                    RequirementCheck {
                        requirement_id: requirement.id.clone(),
                        requirement_name: requirement.name.clone(),
                        status: "missing".to_string(),
                        expected: expected_text(requirement),
                        actual: String::new(),
                        unit: requirement.unit.clone(),
                        explanation: if requirement.mandatory {
                            "product page does not state this mandatory requirement".to_string()
                        } else {
                            "product page does not state this requirement".to_string()
                        },
                    },
                    0.0,
                )
            }
        };
        weighted_sum += weight * quality;
        weight_total += weight;
        checks.push(check);
    }

    let technical_score = if weight_total > 0.0 {
        (weighted_sum / weight_total).clamp(0.0, 1.0)
    } else {
        1.0
    };

    let mandatory_mismatch = checks.iter().any(|check| {
        check.status == "mismatch" && requirement_is_mandatory(requirements, &check.requirement_id)
    });
    let mandatory_missing = checks.iter().any(|check| {
        check.status == "missing" && requirement_is_mandatory(requirements, &check.requirement_id)
    });
    let any_tolerance = checks.iter().any(|check| check.status == "tolerance");
    let any_soft_issue = checks
        .iter()
        .any(|check| matches!(check.status.as_str(), "partial" | "missing" | "tolerance"));

    let verdict = if mandatory_mismatch {
        MatchVerdict::Mismatch
    } else if mandatory_missing {
        MatchVerdict::Insufficient
    } else if any_tolerance {
        MatchVerdict::Tolerance
    } else if any_soft_issue {
        MatchVerdict::Partial
    } else {
        MatchVerdict::Full
    };

    let eligible = matches!(
        verdict,
        MatchVerdict::Full | MatchVerdict::Tolerance | MatchVerdict::Partial
    );

    MatchResult {
        component_id: component_id.to_string(),
        product_id: product.product_id.clone(),
        verdict: verdict as i32,
        eligible,
        technical_score,
        checks,
        missing_specs,
    }
}

fn requirement_is_mandatory(requirements: &[Requirement], requirement_id: &str) -> bool {
    requirements
        .iter()
        .find(|requirement| requirement.id == requirement_id)
        .is_some_and(|requirement| requirement.mandatory)
}

/// Evaluate one requirement against a spec. Returns the check and its
/// quality contribution (1.0 match, 0.9 tolerance, 0.5 partial, 0.0 else).
fn evaluate(requirement: &Requirement, spec: &SpecValue) -> (RequirementCheck, f64) {
    let expected = expected_text(requirement);
    let actual = actual_text(spec);
    let unit_conflict = !requirement.unit.trim().is_empty()
        && !spec.unit.trim().is_empty()
        && normalize(&requirement.unit) != normalize(&spec.unit);

    let (status, quality, explanation) = if unit_conflict {
        (
            "mismatch",
            0.0,
            format!(
                "unit differs (tender '{}' vs product '{}'); no unit conversion is performed",
                requirement.unit.trim(),
                spec.unit.trim()
            ),
        )
    } else {
        match requirement.data_type.as_str() {
            "NUMBER" | "DIMENSION" => compare_numeric(requirement, spec, &expected, &actual),
            "RANGE" => compare_range(requirement, spec, &expected, &actual),
            "BOOLEAN" => compare_boolean(requirement, spec, &expected, &actual),
            "ENUM" => compare_enum(requirement, spec, &expected, &actual),
            // STRING | STANDARD | MATERIAL | MODEL | anything unknown
            _ => compare_text(&expected, &actual),
        }
    };

    (
        RequirementCheck {
            requirement_id: requirement.id.clone(),
            requirement_name: requirement.name.clone(),
            status: status.to_string(),
            expected,
            actual,
            unit: requirement.unit.clone(),
            explanation,
        },
        quality,
    )
}

fn compare_numeric(
    requirement: &Requirement,
    spec: &SpecValue,
    expected: &str,
    actual: &str,
) -> (&'static str, f64, String) {
    let expected_num = numeric_value(requirement.value_num, &requirement.value_text);
    let actual_num = numeric_value(spec.value_num, &spec.value_text);
    match (expected_num, actual_num) {
        (Some(expected_num), Some(actual_num)) => {
            let diff = (actual_num - expected_num).abs();
            let tolerance = if requirement.tolerance > 0.0 {
                expected_num.abs() * requirement.tolerance
            } else {
                EXACT_EPSILON * expected_num.abs().max(1.0)
            };
            if diff <= EXACT_EPSILON * expected_num.abs().max(1.0) {
                ("match", 1.0, String::new())
            } else if diff <= tolerance {
                (
                    "tolerance",
                    0.9,
                    format!(
                        "value deviates by {:.6} which is within the allowed tolerance {:.4}",
                        diff, tolerance
                    ),
                )
            } else {
                (
                    "mismatch",
                    0.0,
                    format!(
                        "expected {expected} but product states {actual} (deviation {:.6} exceeds tolerance {:.4})",
                        diff, tolerance
                    ),
                )
            }
        }
        _ => (
            "partial",
            0.5,
            format!("cannot compare numeric values: expected '{expected}', product '{actual}'"),
        ),
    }
}

fn compare_range(
    requirement: &Requirement,
    spec: &SpecValue,
    expected: &str,
    actual: &str,
) -> (&'static str, f64, String) {
    let value = numeric_value(spec.value_num, &spec.value_text);
    match value {
        Some(value) => {
            let min = requirement.range_min;
            let max = requirement.range_max;
            let above_min = min == 0.0 || value >= min;
            let below_max = max == 0.0 || value <= max;
            if above_min && below_max {
                ("match", 1.0, String::new())
            } else {
                (
                    "mismatch",
                    0.0,
                    format!(
                        "product value {actual} is outside the required range [{min}, {max}] (tender: {expected})"
                    ),
                )
            }
        }
        None => (
            "partial",
            0.5,
            format!("cannot compare range: product states '{actual}'"),
        ),
    }
}

fn compare_boolean(
    requirement: &Requirement,
    spec: &SpecValue,
    expected: &str,
    actual: &str,
) -> (&'static str, f64, String) {
    let expected_bool = parse_bool(&requirement.value_text);
    let actual_bool = parse_bool(&spec.value_text);
    match (expected_bool, actual_bool) {
        (Some(expected_bool), Some(actual_bool)) if expected_bool == actual_bool => {
            ("match", 1.0, String::new())
        }
        (Some(_), Some(_)) => (
            "mismatch",
            0.0,
            format!("expected {expected} but product states {actual}"),
        ),
        _ => (
            "partial",
            0.5,
            format!("cannot interpret boolean: expected '{expected}', product '{actual}'"),
        ),
    }
}

fn compare_enum(
    requirement: &Requirement,
    spec: &SpecValue,
    expected: &str,
    actual: &str,
) -> (&'static str, f64, String) {
    let actual_norm = normalize(&spec.value_text);
    if actual_norm.is_empty() {
        return (
            "partial",
            0.5,
            "product page states an empty value for an enumerated requirement".to_string(),
        );
    }
    let allowed: Vec<String> = requirement
        .allowed_values
        .iter()
        .map(|value| normalize(value))
        .collect();
    if allowed.is_empty() {
        return compare_text(expected, actual);
    }
    if allowed.iter().any(|value| value == &actual_norm) {
        ("match", 1.0, String::new())
    } else {
        (
            "mismatch",
            0.0,
            format!(
                "product value '{actual}' is not one of the allowed values [{}]",
                requirement.allowed_values.join(", ")
            ),
        )
    }
}

fn compare_text(expected: &str, actual: &str) -> (&'static str, f64, String) {
    let expected_norm = normalize(expected);
    let actual_norm = normalize(actual);
    if expected_norm.is_empty() {
        return (
            "partial",
            0.5,
            "tender requirement has no comparable value".to_string(),
        );
    }
    if actual_norm.is_empty() {
        return (
            "partial",
            0.5,
            "product page states an empty value".to_string(),
        );
    }
    if expected_norm == actual_norm {
        return ("match", 1.0, String::new());
    }
    if actual_norm.contains(&expected_norm) || expected_norm.contains(&actual_norm) {
        return (
            "partial",
            0.5,
            format!("'{actual}' only partially matches the expected '{expected}'"),
        );
    }
    (
        "mismatch",
        0.0,
        format!("expected '{expected}' but product states '{actual}'"),
    )
}

/// Exact normalized name match first, then a conservative substring match.
fn find_spec<'a>(specs: &'a [SpecValue], name: &str) -> Option<&'a SpecValue> {
    let wanted = normalize(name);
    if wanted.is_empty() {
        return None;
    }
    specs
        .iter()
        .find(|spec| normalize(&spec.name) == wanted)
        .or_else(|| {
            specs.iter().find(|spec| {
                let spec_name = normalize(&spec.name);
                !spec_name.is_empty()
                    && (spec_name.contains(&wanted) || wanted.contains(&spec_name))
            })
        })
}

/// Lowercase, strip diacritics that commonly differ (ё→е), collapse
/// whitespace and drop trailing punctuation — deterministic and locale-free.
pub fn normalize(text: &str) -> String {
    text.to_lowercase()
        .replace('ё', "е")
        .chars()
        .map(|ch| if ch.is_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn numeric_value(num: f64, text: &str) -> Option<f64> {
    if num != 0.0 {
        return Some(num);
    }
    let cleaned: String = text
        .trim()
        .replace(' ', "")
        .replace(',', ".")
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || *ch == '.' || *ch == '-' || *ch == '+')
        .collect();
    cleaned.parse::<f64>().ok()
}

fn parse_bool(text: &str) -> Option<bool> {
    match normalize(text).as_str() {
        "true" | "yes" | "да" | "есть" | "1" | "on" | "включен" | "включено" | "есть в наличии" => {
            Some(true)
        }
        "false" | "no" | "нет" | "0" | "off" | "отсутствует" | "выключен" | "выключено" => {
            Some(false)
        }
        _ => None,
    }
}

fn expected_text(requirement: &Requirement) -> String {
    if !requirement.value_text.trim().is_empty() {
        return with_unit(&requirement.value_text, &requirement.unit);
    }
    if requirement.data_type == "RANGE" {
        return format!(
            "[{}, {}] {}",
            requirement.range_min,
            requirement.range_max,
            requirement.unit.trim()
        )
        .trim()
        .to_string();
    }
    if requirement.value_num != 0.0 {
        return with_unit(&requirement.value_num.to_string(), &requirement.unit);
    }
    if !requirement.allowed_values.is_empty() {
        return requirement.allowed_values.join(" | ");
    }
    String::new()
}

fn actual_text(spec: &SpecValue) -> String {
    if !spec.value_text.trim().is_empty() {
        return with_unit(&spec.value_text, &spec.unit);
    }
    if spec.value_num != 0.0 {
        return with_unit(&spec.value_num.to_string(), &spec.unit);
    }
    String::new()
}

fn with_unit(value: &str, unit: &str) -> String {
    let value = value.trim();
    let unit = unit.trim();
    if unit.is_empty() || value.ends_with(unit) {
        value.to_string()
    } else {
        format!("{value} {unit}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn requirement(name: &str, data_type: &str, value: &str, mandatory: bool) -> Requirement {
        Requirement {
            id: format!("req-{name}"),
            name: name.to_string(),
            value_text: value.to_string(),
            value_num: 0.0,
            unit: String::new(),
            data_type: data_type.to_string(),
            mandatory,
            range_min: 0.0,
            range_max: 0.0,
            tolerance: 0.0,
            allowed_values: Vec::new(),
            source: "document".to_string(),
            confidence: 1.0,
        }
    }

    fn spec(name: &str, value: &str) -> SpecValue {
        SpecValue {
            id: format!("spec-{name}"),
            name: name.to_string(),
            value_text: value.to_string(),
            value_num: 0.0,
            unit: String::new(),
            data_type: "STRING".to_string(),
            source_id: "src-1".to_string(),
            source_snippet: value.to_string(),
            confidence: 0.9,
        }
    }

    fn product(specs: Vec<SpecValue>) -> ProductCandidate {
        ProductCandidate {
            product_id: "p-1".to_string(),
            specs,
        }
    }

    #[test]
    fn exact_text_match_is_full_and_eligible() {
        let requirements = vec![requirement(
            "Материал",
            "MATERIAL",
            "нержавеющая сталь",
            true,
        )];
        let result = match_product(
            "c-1",
            &requirements,
            &product(vec![spec("материал", "Нержавеющая сталь")]),
        );
        assert_eq!(result.verdict, MatchVerdict::Full as i32);
        assert!(result.eligible);
        assert_eq!(result.technical_score, 1.0);
        assert!(result.missing_specs.is_empty());
    }

    #[test]
    fn mandatory_missing_spec_is_insufficient_not_a_match() {
        let requirements = vec![requirement("Мощность", "NUMBER", "5", true)];
        let result = match_product("c-1", &requirements, &product(vec![]));
        assert_eq!(result.verdict, MatchVerdict::Insufficient as i32);
        assert!(!result.eligible);
        assert_eq!(result.missing_specs, vec!["Мощность".to_string()]);
    }

    #[test]
    fn mandatory_mismatch_is_rejected_even_if_others_match() {
        let requirements = vec![
            requirement("Материал", "MATERIAL", "сталь", true),
            requirement("Цвет", "STRING", "синий", false),
        ];
        let result = match_product(
            "c-1",
            &requirements,
            &product(vec![spec("материал", "алюминий"), spec("цвет", "синий")]),
        );
        assert_eq!(result.verdict, MatchVerdict::Mismatch as i32);
        assert!(!result.eligible);
        assert!(result.technical_score < 1.0);
    }

    #[test]
    fn numeric_tolerance_verdict() {
        let mut length = requirement("Длина", "NUMBER", "100", true);
        length.unit = "мм".to_string();
        length.tolerance = 0.05;
        let mut product_spec = spec("длина", "104");
        product_spec.unit = "мм".to_string();

        let result = match_product("c-1", &[length], &product(vec![product_spec]));
        assert_eq!(result.verdict, MatchVerdict::Tolerance as i32);
        assert!(result.eligible);
        assert!(result.checks[0].explanation.contains("tolerance"));
    }

    #[test]
    fn numeric_outside_tolerance_is_mismatch() {
        let mut length = requirement("Длина", "NUMBER", "100", true);
        length.unit = "мм".to_string();
        length.tolerance = 0.05;
        let mut product_spec = spec("длина", "120");
        product_spec.unit = "мм".to_string();
        let result = match_product("c-1", &[length], &product(vec![product_spec]));
        assert_eq!(result.verdict, MatchVerdict::Mismatch as i32);
    }

    #[test]
    fn unit_conflict_is_mismatch_with_explanation() {
        let mut length = requirement("Длина", "NUMBER", "100", true);
        length.unit = "мм".to_string();
        let mut product_spec = spec("длина", "100");
        product_spec.unit = "см".to_string();
        let result = match_product("c-1", &[length], &product(vec![product_spec]));
        assert_eq!(result.verdict, MatchVerdict::Mismatch as i32);
        assert!(result.checks[0].explanation.contains("unit differs"));
    }

    #[test]
    fn range_check() {
        let mut pressure = requirement("Давление", "RANGE", "", true);
        pressure.range_min = 1.0;
        pressure.range_max = 10.0;
        let result = match_product(
            "c-1",
            &[pressure.clone()],
            &product(vec![spec("давление", "6")]),
        );
        assert_eq!(result.verdict, MatchVerdict::Full as i32);

        let result = match_product("c-1", &[pressure], &product(vec![spec("давление", "12")]));
        assert_eq!(result.verdict, MatchVerdict::Mismatch as i32);
    }

    #[test]
    fn enum_check_uses_allowed_values() {
        let mut kind = requirement("Тип", "ENUM", "шаровой", true);
        kind.allowed_values = vec!["шаровой".to_string(), "задвижка".to_string()];
        let result = match_product(
            "c-1",
            &[kind.clone()],
            &product(vec![spec("тип", "Шаровой")]),
        );
        assert_eq!(result.verdict, MatchVerdict::Full as i32);

        let result = match_product("c-1", &[kind], &product(vec![spec("тип", "дисковый")]));
        assert_eq!(result.verdict, MatchVerdict::Mismatch as i32);
    }

    #[test]
    fn boolean_check_understands_russian() {
        let flag = requirement("Взрывозащита", "BOOLEAN", "да", true);
        let result = match_product("c-1", &[flag], &product(vec![spec("взрывозащита", "есть")]));
        assert_eq!(result.verdict, MatchVerdict::Full as i32);
    }

    #[test]
    fn non_mandatory_missing_is_partial_but_eligible() {
        let requirements = vec![
            requirement("Материал", "MATERIAL", "сталь", true),
            requirement("Гарантия", "STRING", "24 месяца", false),
        ];
        let result = match_product(
            "c-1",
            &requirements,
            &product(vec![spec("материал", "сталь")]),
        );
        assert_eq!(result.verdict, MatchVerdict::Partial as i32);
        assert!(result.eligible);
    }

    #[test]
    fn component_matching_skips_products_without_id() {
        let requirements = vec![requirement("Материал", "MATERIAL", "сталь", true)];
        let results = match_component(
            "c-1",
            &requirements,
            &[
                product(vec![spec("материал", "сталь")]),
                ProductCandidate {
                    product_id: String::new(),
                    specs: vec![],
                },
            ],
        );
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn normalize_is_stable() {
        assert_eq!(normalize("  Нержавеющая   СТАЛЬ "), "нержавеющая сталь");
        assert_eq!(normalize("Сталь, ёмкость"), "сталь емкость");
        assert_eq!(normalize(""), "");
    }
}
