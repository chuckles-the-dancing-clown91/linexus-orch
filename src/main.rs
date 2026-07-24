//! # Linexus Orchestrator — The Nervous System
//!
//! Two planners live here:
//!   * the **economic** demand/supply router and KPI rebalancer (`allocation`,
//!     `rebalancer`) that matches labor to nodes for the Vicinagora economy via
//!     `linexus-core`; and
//!   * the **operational** transaction planner (`plan`, `api`) that resolves
//!     infrastructure intents from Nexus into executable agent plans.
//!
//! The binary runs the operational planner's HTTP service; the economic modules
//! remain available as a library for the economy line.

#![allow(dead_code)]

mod allocation;
mod api;
mod plan;
mod rebalancer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_target(true).init();

    let bind = std::env::var("ORCH_BIND").unwrap_or_else(|_| "0.0.0.0:5152".to_string());
    let token = std::env::var("ORCH_SERVICE_TOKEN")
        .ok()
        .filter(|s| !s.is_empty());

    tracing::info!("=== LINEXUS ORCH — Orchestrator ===");
    tracing::info!("Transaction Planner: STARTING");
    if token.is_none() {
        tracing::warn!("ORCH_SERVICE_TOKEN unset — HTTP auth disabled (development mode)");
    }

    let app = api::router(api::AppState { token });
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!(bind = %bind, "orchestrator HTTP listening");
    axum::serve(listener, app).await?;

    Ok(())
}
