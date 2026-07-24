//! HTTP surface for the orchestrator's planning engine.
//!
//! Nexus is the expected caller: it forwards a task intent and receives an
//! ordered, executable [`TransactionPlan`]. Auth is a shared bearer service
//! token (`ORCH_SERVICE_TOKEN`); when unset the service runs open for local
//! development.
//!
//! * `GET  /healthz` — liveness.
//! * `POST /plan`    — resolve an intent into a plan.

use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::json;

use crate::plan::{plan_intent, PlanRequest, PlanResponse};

#[derive(Clone)]
pub struct AppState {
    /// Expected bearer token. `None`/empty disables auth (dev only).
    pub token: Option<String>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/plan", post(plan))
        .with_state(state)
}

async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

async fn plan(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PlanRequest>,
) -> Response {
    if !authorized(&headers, &st.token) {
        return unauthorized();
    }

    let plan = plan_intent(&req);
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
