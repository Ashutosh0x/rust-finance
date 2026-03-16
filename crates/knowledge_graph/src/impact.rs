use std::collections::HashMap;
use crate::graph::FinancialGraph;
use crate::query::{ImpactPath, ImpactStep};

/// Quantifies how a shock to entity A propagates to entity B.
/// Used by ReACT report_agent to score scenario impacts.
pub struct ImpactEngine<'a> {
    graph: &'a FinancialGraph,
}

#[derive(Debug, Clone)]
pub struct ImpactScore {
    pub target_id: String,
    pub target_name: String,
    /// -1.0 (max bearish) to +1.0 (max bullish)
    pub score: f64,
    /// Confidence in the score based on path weight quality
    pub confidence: f64,
    pub path: ImpactPath,
    pub explanation: String,
}

impl<'a> ImpactEngine<'a> {
    pub fn new(graph: &'a FinancialGraph) -> Self {
        Self { graph }
    }

    /// Score the impact of a shock to `source_id` on all reachable entities.
    /// Returns sorted by absolute impact magnitude.
    pub fn propagate_shock(
        &self,
        source_id: &str,
        shock_magnitude: f64,  // -1.0 to +1.0
        max_hops: usize,
    ) -> Vec<ImpactScore> {
        let mut scores: Vec<ImpactScore> = Vec::new();
        let mut visited = HashMap::new();

        self.dfs_propagate(
            source_id,
            source_id,
            shock_magnitude,
            1.0,
            max_hops,
            0,
            &mut visited,
            &mut scores,
            vec![],
        );

        scores.sort_by(|a, b| {
            b.score.abs().partial_cmp(&a.score.abs()).unwrap_or(std::cmp::Ordering::Equal)
        });
        scores
    }

    /// Score the specific impact of source on one target — returns None if not connected.
    pub fn score_pair(
        &self,
        source_id: &str,
        target_id: &str,
        shock_magnitude: f64,
    ) -> Option<ImpactScore> {
        self.propagate_shock(source_id, shock_magnitude, 3)
            .into_iter()
            .find(|s| s.target_id == target_id)
    }

    /// Human-readable impact table for Dexter AI prompt
    pub fn impact_table(
        &self,
        source_id: &str,
        shock_magnitude: f64,
        top_n: usize,
    ) -> String {
        let scores = self.propagate_shock(source_id, shock_magnitude, 2);
        let source_name = self.graph.get_entity(source_id)
            .map(|e| e.name.as_str())
            .unwrap_or(source_id);

        let dir = if shock_magnitude > 0.0 { "rises" } else { "falls" };
        let mut lines = vec![format!(
            "Impact analysis: if {} {} by {:.0}%:",
            source_name, dir, shock_magnitude.abs() * 100.0
        )];

        for s in scores.iter().take(top_n) {
            let arrow = if s.score > 0.0 { "↑" } else { "↓" };
            lines.push(format!(
                "  {} {} {:.0}% impact on {} (confidence: {:.0}%)",
                arrow,
                if s.score > 0.0 { "bullish" } else { "bearish" },
                s.score.abs() * 100.0,
                s.target_name,
                s.confidence * 100.0,
            ));
        }

        lines.join("\n")
    }

    // ── DFS propagation ──────────────────────────────────────────────

    #[allow(clippy::too_many_arguments)]
    fn dfs_propagate(
        &self,
        current_id: &str,
        origin_id: &str,
        current_score: f64,
        path_weight: f64,
        max_hops: usize,
        depth: usize,
        visited: &mut HashMap<String, f64>,
        results: &mut Vec<ImpactScore>,
        current_path: Vec<ImpactStep>,
    ) {
        if depth >= max_hops {
            return;
        }

        let affected = self.graph.affected_by(current_id);

        for (target, rel) in affected {
            if target.id == origin_id {
                continue; // avoid cycles back to source
            }

            // Score attenuation per hop
            let hop_sign = rel.kind.propagation_sign();
            let hop_score = current_score * rel.weight * hop_sign.signum().max(1.0) * hop_sign;
            let hop_weight = path_weight * rel.weight;

            // Only record if signal is above noise threshold
            if hop_score.abs() < 0.01 {
                continue;
            }

            let mut new_path = current_path.clone();
            new_path.push(ImpactStep {
                from: current_id.to_string(),
                to: target.id.clone(),
                relationship: rel.kind.label().to_string(),
                weight: rel.weight,
            });

            // Use strongest path score if already visited
            let existing = visited.entry(target.id.clone()).or_insert(0.0);
            if hop_score.abs() > existing.abs() {
                *existing = hop_score;

                let explanation = format!(
                    "{} → {} via '{}' ({}): {:.0}% {} impact",
                    current_id,
                    target.id,
                    rel.kind.label(),
                    rel.description,
                    hop_score.abs() * 100.0,
                    if hop_score > 0.0 { "bullish" } else { "bearish" }
                );

                results.retain(|r| r.target_id != target.id);
                results.push(ImpactScore {
                    target_id: target.id.clone(),
                    target_name: target.name.clone(),
                    score: hop_score,
                    confidence: hop_weight,
                    path: ImpactPath {
                        steps: new_path.clone(),
                        total_weight: hop_weight,
                        direction: hop_score.signum(),
                    },
                    explanation,
                });
            }

            // Recurse
            self.dfs_propagate(
                &target.id,
                origin_id,
                hop_score,
                hop_weight,
                max_hops,
                depth + 1,
                visited,
                results,
                new_path,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::seed_graph;

    #[test]
    fn oil_shock_hits_airlines_negatively() {
        let graph = seed_graph();
        let engine = ImpactEngine::new(&graph);

        let scores = engine.propagate_shock("OIL_WTI", 0.20, 2);
        let airline_score = scores.iter().find(|s| s.target_id == "AIRLINES");

        // Oil up 20% should be bearish for airlines
        if let Some(score) = airline_score {
            assert!(score.score < 0.0, "Oil up should be bearish for airlines, got {}", score.score);
        }
    }

    #[test]
    fn impact_table_returns_string() {
        let graph = seed_graph();
        let engine = ImpactEngine::new(&graph);
        let table = engine.impact_table("FED_FUNDS_RATE", 0.75, 5);
        assert!(table.contains("FED_FUNDS_RATE") || table.contains("Fed Funds Rate"));
    }
}
