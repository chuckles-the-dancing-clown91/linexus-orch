//! KPI Rebalancer
use linexus_core::jobs::{JobEngine, KpiMetric, MaintenanceJob};
use std::collections::HashMap;
use uuid::Uuid;

pub struct Rebalancer;

impl Rebalancer {
    pub fn run_epoch_rebalance(node_metrics: &HashMap<Uuid, HashMap<String, KpiMetric>>) -> Vec<MaintenanceJob> {
        let mut all_jobs = Vec::new();
        for (node_id, metrics) in node_metrics {
            all_jobs.extend(JobEngine::evaluate_and_rebalance_node(*node_id, metrics));
        }
        tracing::info!("Epoch rebalance: {} jobs across {} nodes", all_jobs.len(), node_metrics.len());
        all_jobs
    }
}
