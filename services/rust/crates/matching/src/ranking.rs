//! Explainable weighted ranking.
//!
//! Every factor is normalized to `0..1` and reported in the breakdown with
//! its weight and a note. Unknown factors score at the floor (0) with an
//! explicit note — the engine never fabricates a price, a lead time or a
//! supplier rating to make a product look better.

use crate::tender::matching::v1::{
    RankOffersRequest, RankedInput, RankedOffer, RankingWeights, ScoreComponent,
};

pub const DEFAULT_TECHNICAL_FIT: f64 = 0.40;
pub const DEFAULT_PRICE: f64 = 0.25;
pub const DEFAULT_DELIVERY: f64 = 0.20;
pub const DEFAULT_SUPPLIER_RELIABILITY: f64 = 0.10;
pub const DEFAULT_DATA_FRESHNESS: f64 = 0.05;

/// Freshness half-life: data aging to 30 days normalizes to 0.5.
const FRESHNESS_HALF_LIFE_DAYS: f64 = 30.0;

#[derive(Debug, Clone, Copy)]
pub struct Weights {
    pub technical_fit: f64,
    pub price: f64,
    pub delivery: f64,
    pub supplier_reliability: f64,
    pub data_freshness: f64,
}

impl Weights {
    pub fn defaults() -> Self {
        Self {
            technical_fit: DEFAULT_TECHNICAL_FIT,
            price: DEFAULT_PRICE,
            delivery: DEFAULT_DELIVERY,
            supplier_reliability: DEFAULT_SUPPLIER_RELIABILITY,
            data_freshness: DEFAULT_DATA_FRESHNESS,
        }
    }

    /// Zero-filled request means "use configured defaults"; any provided
    /// values are clamped at zero and renormalized to sum to 1.
    pub fn resolve(requested: &RankingWeights) -> Self {
        let raw = [
            requested.technical_fit,
            requested.price,
            requested.delivery,
            requested.supplier_reliability,
            requested.data_freshness,
        ];
        if raw.iter().all(|value| *value <= 0.0) {
            return Self::defaults();
        }
        let sum: f64 = raw.iter().map(|value| value.max(0.0)).sum();
        if sum <= 0.0 {
            return Self::defaults();
        }
        Self {
            technical_fit: raw[0].max(0.0) / sum,
            price: raw[1].max(0.0) / sum,
            delivery: raw[2].max(0.0) / sum,
            supplier_reliability: raw[3].max(0.0) / sum,
            data_freshness: raw[4].max(0.0) / sum,
        }
    }

    pub fn factors(&self) -> [(&'static str, f64); 5] {
        [
            ("technical_fit", self.technical_fit),
            ("price", self.price),
            ("delivery", self.delivery),
            ("supplier_reliability", self.supplier_reliability),
            ("data_freshness", self.data_freshness),
        ]
    }
}

/// Rank the eligible offers of one component, best first.
pub fn rank_component(request: &RankOffersRequest) -> Vec<RankedOffer> {
    let weights = Weights::resolve(
        request
            .weights
            .as_ref()
            .unwrap_or(&RankingWeights::default()),
    );
    let eligible: Vec<&RankedInput> = request
        .offers
        .iter()
        .filter(|offer| {
            !offer.product_id.trim().is_empty()
                && offer.r#match.as_ref().is_some_and(|m| m.eligible)
        })
        .collect();
    if eligible.is_empty() {
        return Vec::new();
    }

    let cheapest = eligible
        .iter()
        .filter(|offer| offer.has_price && offer.price > 0.0)
        .map(|offer| offer.price)
        .fold(None::<f64>, |best, price| {
            Some(best.map_or(price, |b| b.min(price)))
        });
    let fastest = eligible
        .iter()
        .filter(|offer| offer.has_lead_time)
        .map(|offer| offer.lead_time_days.max(0))
        .min();

    let mut ranked: Vec<RankedOffer> = eligible
        .iter()
        .map(|offer| rank_offer(offer, &weights, cheapest, fastest))
        .collect();

    ranked.sort_by(|left, right| {
        right
            .total_score
            .partial_cmp(&left.total_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.product_id.cmp(&right.product_id))
    });
    ranked
}

fn rank_offer(
    offer: &RankedInput,
    weights: &Weights,
    cheapest: Option<f64>,
    fastest: Option<i32>,
) -> RankedOffer {
    let mut breakdown: Vec<ScoreComponent> = Vec::with_capacity(5);

    let technical = offer
        .r#match
        .as_ref()
        .map_or(0.0, |matched| matched.technical_score.clamp(0.0, 1.0));
    breakdown.push(factor(
        "technical_fit",
        weights.technical_fit,
        technical,
        "",
    ));

    let (price_value, price_note) = match cheapest {
        Some(cheapest) if offer.has_price && offer.price > 0.0 => {
            let normalized = (cheapest / offer.price).clamp(0.0, 1.0);
            let note = if (offer.price - cheapest).abs() < f64::EPSILON {
                "lowest price among the considered offers".to_string()
            } else {
                format!(
                    "price is {:.1}% above the cheapest offer",
                    (offer.price / cheapest - 1.0) * 100.0
                )
            };
            (normalized, note)
        }
        _ if offer.has_price && offer.price > 0.0 => {
            (1.0, "the only priced offer; normalized to 1".to_string())
        }
        _ => (0.0, "price unknown — scored at floor (0)".to_string()),
    };
    breakdown.push(factor("price", weights.price, price_value, &price_note));

    let (delivery_value, delivery_note) = match fastest {
        Some(fastest) if offer.has_lead_time => {
            let days = offer.lead_time_days.max(0);
            let normalized = if days <= 0 {
                1.0
            } else {
                (f64::from(fastest) / f64::from(days)).clamp(0.0, 1.0)
            };
            (
                normalized,
                format!("lead time {days} days (fastest {fastest} days)"),
            )
        }
        _ => (0.0, "lead time unknown — scored at floor (0)".to_string()),
    };
    breakdown.push(factor(
        "delivery",
        weights.delivery,
        delivery_value,
        &delivery_note,
    ));

    let (reliability_value, reliability_note) = if offer.has_supplier_reliability {
        (
            offer.supplier_reliability.clamp(0.0, 1.0),
            format!("supplier reliability {:.2}", offer.supplier_reliability),
        )
    } else {
        (
            0.0,
            "supplier reliability unknown — scored at floor (0)".to_string(),
        )
    };
    breakdown.push(factor(
        "supplier_reliability",
        weights.supplier_reliability,
        reliability_value,
        &reliability_note,
    ));

    let (freshness_value, freshness_note) = if offer.data_age_days <= 0 {
        (1.0, "data retrieved today".to_string())
    } else {
        let age = f64::from(offer.data_age_days);
        (
            1.0 / (1.0 + age / FRESHNESS_HALF_LIFE_DAYS),
            format!("data is {} days old", offer.data_age_days),
        )
    };
    breakdown.push(factor(
        "data_freshness",
        weights.data_freshness,
        freshness_value,
        &freshness_note,
    ));

    let total_score: f64 = breakdown.iter().map(|component| component.weighted).sum();
    let total_score = total_score.clamp(0.0, 1.0);
    let summary = summarize(&breakdown, total_score);

    RankedOffer {
        product_id: offer.product_id.clone(),
        total_score,
        breakdown,
        summary,
    }
}

fn factor(name: &str, weight: f64, normalized: f64, note: &str) -> ScoreComponent {
    let normalized = normalized.clamp(0.0, 1.0);
    ScoreComponent {
        factor: name.to_string(),
        weight,
        normalized,
        weighted: weight * normalized,
        note: note.to_string(),
    }
}

/// One-line explanation naming the strongest and weakest factor.
fn summarize(breakdown: &[ScoreComponent], total: f64) -> String {
    let strongest = breakdown.iter().max_by(|left, right| {
        left.weighted
            .partial_cmp(&right.weighted)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let weakest = breakdown
        .iter()
        .filter(|component| component.weight > 0.0)
        .min_by(|left, right| {
            left.normalized
                .partial_cmp(&right.normalized)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    match (strongest, weakest) {
        (Some(strongest), Some(weakest)) => format!(
            "total {total:.3}; strongest factor {} ({:.3}), weakest {} ({:.3})",
            strongest.factor, strongest.weighted, weakest.factor, weakest.normalized
        ),
        _ => format!("total {total:.3}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tender::matching::v1::MatchResult;

    fn offer(
        product_id: &str,
        technical: f64,
        price: Option<f64>,
        lead: Option<i32>,
        age_days: i32,
    ) -> RankedInput {
        RankedInput {
            product_id: product_id.to_string(),
            r#match: Some(MatchResult {
                component_id: "c-1".to_string(),
                product_id: product_id.to_string(),
                verdict: 1,
                eligible: true,
                technical_score: technical,
                checks: Vec::new(),
                missing_specs: Vec::new(),
            }),
            has_price: price.is_some(),
            price: price.unwrap_or(0.0),
            currency: "RUB".to_string(),
            has_lead_time: lead.is_some(),
            lead_time_days: lead.unwrap_or(0),
            has_supplier_reliability: false,
            supplier_reliability: 0.0,
            data_age_days: age_days,
        }
    }

    fn request(offers: Vec<RankedInput>) -> RankOffersRequest {
        RankOffersRequest {
            component_id: "c-1".to_string(),
            offers,
            weights: None,
        }
    }

    #[test]
    fn defaults_are_used_when_weights_are_zero() {
        let weights = Weights::resolve(&RankingWeights::default());
        assert!((weights.technical_fit - 0.40).abs() < 1e-9);
        assert!((weights.price - 0.25).abs() < 1e-9);
        assert!((weights.data_freshness - 0.05).abs() < 1e-9);
    }

    #[test]
    fn custom_weights_are_normalized() {
        let weights = Weights::resolve(&RankingWeights {
            technical_fit: 2.0,
            price: 2.0,
            delivery: 0.0,
            supplier_reliability: 0.0,
            data_freshness: 0.0,
        });
        assert!((weights.technical_fit - 0.5).abs() < 1e-9);
        assert!((weights.price - 0.5).abs() < 1e-9);
        assert!((weights.delivery).abs() < 1e-9);
    }

    #[test]
    fn cheaper_and_faster_offer_ranks_first() {
        let ranked = rank_component(&request(vec![
            offer("expensive", 1.0, Some(200.0), Some(30), 0),
            offer("cheap", 1.0, Some(100.0), Some(5), 0),
        ]));
        assert_eq!(ranked[0].product_id, "cheap");
        assert!(ranked[0].total_score > ranked[1].total_score);
    }

    #[test]
    fn unknown_price_scores_at_floor_with_note() {
        let ranked = rank_component(&request(vec![
            offer("priced", 1.0, Some(100.0), Some(10), 0),
            offer("unknown", 1.0, None, None, 0),
        ]));
        let unknown = ranked
            .iter()
            .find(|offer| offer.product_id == "unknown")
            .expect("ranked offer present");
        let price = unknown
            .breakdown
            .iter()
            .find(|component| component.factor == "price")
            .expect("price factor");
        assert_eq!(price.normalized, 0.0);
        assert!(price.note.contains("unknown"));
        assert_eq!(ranked[0].product_id, "priced");
    }

    #[test]
    fn ineligible_matches_are_excluded() {
        let mut input = offer("bad", 0.0, Some(1.0), Some(1), 0);
        if let Some(matched) = input.r#match.as_mut() {
            matched.eligible = false;
        }
        let ranked = rank_component(&request(vec![
            input,
            offer("good", 1.0, Some(10.0), Some(3), 0),
        ]));
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].product_id, "good");
    }

    #[test]
    fn freshness_decays_with_age() {
        let ranked = rank_component(&request(vec![
            offer("fresh", 1.0, Some(100.0), Some(10), 0),
            offer("stale", 1.0, Some(100.0), Some(10), 30),
        ]));
        let fresh = ranked.iter().find(|o| o.product_id == "fresh").unwrap();
        let stale = ranked.iter().find(|o| o.product_id == "stale").unwrap();
        let fresh_factor = fresh
            .breakdown
            .iter()
            .find(|c| c.factor == "data_freshness")
            .unwrap();
        let stale_factor = stale
            .breakdown
            .iter()
            .find(|c| c.factor == "data_freshness")
            .unwrap();
        assert!((fresh_factor.normalized - 1.0).abs() < 1e-9);
        assert!((stale_factor.normalized - 0.5).abs() < 1e-9);
        assert!(fresh.total_score > stale.total_score);
    }

    #[test]
    fn ties_break_by_product_id_for_determinism() {
        let ranked = rank_component(&request(vec![
            offer("b", 1.0, Some(100.0), Some(5), 0),
            offer("a", 1.0, Some(100.0), Some(5), 0),
        ]));
        assert_eq!(ranked[0].product_id, "a");
        assert_eq!(ranked[1].product_id, "b");
    }

    #[test]
    fn breakdown_weights_sum_to_total() {
        let ranked = rank_component(&request(vec![offer("p", 0.8, Some(90.0), Some(7), 5)]));
        let offer = &ranked[0];
        let sum: f64 = offer
            .breakdown
            .iter()
            .map(|component| component.weighted)
            .sum();
        assert!((sum - offer.total_score).abs() < 1e-9);
        assert!(!offer.summary.is_empty());
    }

    #[test]
    fn single_priced_offer_normalizes_to_one() {
        let ranked = rank_component(&request(vec![offer("only", 1.0, Some(500.0), Some(10), 0)]));
        let price = ranked[0]
            .breakdown
            .iter()
            .find(|c| c.factor == "price")
            .unwrap();
        assert_eq!(price.normalized, 1.0);
    }
}
