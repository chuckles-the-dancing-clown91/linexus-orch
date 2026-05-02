//! # Linexus Orchestrator — The Nervous System

mod allocation;
mod rebalancer;

use tracing_subscriber;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_target(true).init();
    tracing::info!("=== LINEXUS ORCH — Orchestrator ===");
    tracing::info!("Demand/Supply Engine: ACTIVE");
    tracing::info!("2-Year KPI Rebalancer: ARMED");

    use linexus_core::jobs::{JobEngine, KpiMetric};
    use std::collections::HashMap;
    let metrics = HashMap::from([("solar_efficiency".into(), KpiMetric { metric_name: "solar_efficiency".into(), current_value: 0.70, target_value: 0.90, epoch_degradation_rate: 0.10 })]);
    let jobs = JobEngine::evaluate_and_rebalance_node(uuid::Uuid::new_v4(), &metrics);
    tracing::info!("Generated {} maintenance jobs from KPI drift", jobs.len());
}
