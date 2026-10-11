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
//!
//! `LINEXUS_ENV=production` marks a real deployment: the service then refuses
//! to start without `ORCH_SERVICE_TOKEN`, where development only warns.

#![allow(dead_code)]

mod allocation;
mod api;
mod metrics;
mod plan;
mod rebalancer;

/// Environment variable naming the deployment environment.
const ENV_LINEXUS_ENV: &str = "LINEXUS_ENV";

/// Whether this is a production deployment (`LINEXUS_ENV=production`).
fn is_production(var: &dyn Fn(&str) -> Option<String>) -> bool {
    var(ENV_LINEXUS_ENV).is_some_and(|v| v.trim().eq_ignore_ascii_case("production"))
}

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
        if is_production(&|k| std::env::var(k).ok()) {
            anyhow::bail!(
                "refusing to start: {ENV_LINEXUS_ENV}=production but ORCH_SERVICE_TOKEN is unset — the planner would accept any caller"
            );
        }
        tracing::warn!("ORCH_SERVICE_TOKEN unset — HTTP auth disabled (development mode)");
    }

    let app = api::router(api::AppState { token });
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!(bind = %bind, "orchestrator HTTP listening");
    axum::serve(listener, app).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_is_only_the_explicit_value() {
        assert!(is_production(&|_| Some("production".into())));
        assert!(is_production(&|_| Some(" Production ".into())));
        assert!(!is_production(&|_| Some("development".into())));
        assert!(!is_production(&|_| None));
    }
}
