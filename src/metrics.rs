//! A few process counters, served on `GET /metrics` in Prometheus text format.
//!
//! No metrics crate: the counters are static atomics and the exporter is a
//! handful of lines, which is all a service with two routes needs.

use std::sync::atomic::{AtomicU64, Ordering};

use axum::{extract::Request, middleware::Next, response::Response};

static HTTP_REQUESTS: AtomicU64 = AtomicU64::new(0);
static PLANS: AtomicU64 = AtomicU64::new(0);
static PLAN_FAILURES: AtomicU64 = AtomicU64::new(0);

/// Count a planned intent.
pub fn plan_ok() {
    PLANS.fetch_add(1, Ordering::Relaxed);
}

/// Count an intent the planner rejected.
pub fn plan_failed() {
    PLAN_FAILURES.fetch_add(1, Ordering::Relaxed);
}

/// Middleware counting every request (including `/metrics` itself).
pub async fn count_request(req: Request, next: Next) -> Response {
    HTTP_REQUESTS.fetch_add(1, Ordering::Relaxed);
    next.run(req).await
}

/// The exposition, `text/plain; version=0.0.4`.
pub fn render() -> String {
    let mut out = String::new();
    for (name, help, value) in [
        (
            "orch_http_requests_total",
            "HTTP requests received.",
            HTTP_REQUESTS.load(Ordering::Relaxed),
        ),
        (
            "orch_plans_total",
            "Intents planned successfully.",
            PLANS.load(Ordering::Relaxed),
        ),
        (
            "orch_plan_failures_total",
            "Intents the planner rejected (unknown intent, missing param).",
            PLAN_FAILURES.load(Ordering::Relaxed),
        ),
    ] {
        out.push_str(&format!(
            "# HELP {name} {help}\n# TYPE {name} counter\n{name} {value}\n"
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_prometheus_text() {
        plan_ok();
        plan_failed();
        let text = render();
        assert!(text.contains("# TYPE orch_plans_total counter\n"));
        assert!(text.contains("orch_http_requests_total "));
        let plans: u64 = text
            .lines()
            .find_map(|l| l.strip_prefix("orch_plans_total "))
            .unwrap()
            .parse()
            .unwrap();
        assert!(plans >= 1);
    }
}
