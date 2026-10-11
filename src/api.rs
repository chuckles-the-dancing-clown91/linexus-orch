//! HTTP surface for the orchestrator's planning engine.
//!
//! Nexus is the expected caller: it forwards a task intent and receives an
//! ordered, executable [`TransactionPlan`]. Auth is a shared bearer service
//! token (`ORCH_SERVICE_TOKEN`); when unset the service runs open for local
//! development.
//!
//! * `GET  /healthz` — liveness.
//! * `GET  /metrics` — Prometheus counters (open, like `/healthz`).
//! * `POST /plan`    — resolve an intent into a plan; `400` with
//!   `{"error", "intent"}` when the intent cannot be planned.

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::json;

use crate::metrics;
use crate::plan::{PlanRequest, PlanResponse, plan_intent};

#[derive(Clone)]
pub struct AppState {
    /// Expected bearer token. `None`/empty disables auth (dev only).
    pub token: Option<String>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/metrics", get(metrics_text))
        .route("/plan", post(plan))
        .layer(axum::middleware::from_fn(metrics::count_request))
        .with_state(state)
}

async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

async fn metrics_text() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        metrics::render(),
    )
}

async fn plan(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PlanRequest>,
) -> Response {
    if !authorized(&headers, &st.token) {
        return unauthorized();
    }

    let plan = match plan_intent(&req) {
        Ok(p) => p,
        Err(e) => {
            metrics::plan_failed();
            tracing::warn!(intent = %req.intent, error = %e, "intent rejected");
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": e.to_string(), "intent": req.intent })),
            )
                .into_response();
        }
    };
    metrics::plan_ok();
    let resp = PlanResponse {
        task_id: plan.task_id.clone(),
        status: "planned".to_string(),
        plan,
    };
    tracing::info!(
        task_id = %resp.task_id,
        intent = %req.intent,
        steps = resp.plan.steps.len(),
        "planned intent"
    );
    (StatusCode::OK, Json(resp)).into_response()
}

fn authorized(headers: &HeaderMap, token: &Option<String>) -> bool {
    let expected = match token {
        None => return true,
        Some(t) if t.is_empty() => return true,
        Some(t) => t,
    };
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| constant_time_eq(t.trim(), expected))
        .unwrap_or(false)
}

/// Timing-safe comparison so the token can't be recovered byte-by-byte.
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "error": "missing or invalid bearer token" })),
    )
        .into_response()
}
