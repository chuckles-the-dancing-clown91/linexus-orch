//! Transaction planning for infrastructure (RMM) intents.
//!
//! Daedalus IT submits high-level intents through Nexus — "restart this node",
//! "run this command", "provision this role". The orchestrator resolves each
//! intent into an ordered [`TransactionPlan`] of canonical [`PlanStep`]s that an
//! agent knows how to execute. The step vocabulary here is the contract the
//! agent implements, so it is deliberately small and explicit.
//!
//! This is the operational planner. It is separate from the economic
//! demand/supply routing in `allocation`/`rebalancer`, which plans labor for the
//! Vicinagora economy via `linexus-core`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Canonical step actions. The agent's executor matches on these exact strings.
pub mod action {
    /// Reboot the host. `params`: none.
    pub const SYSTEM_REBOOT: &str = "system.reboot";
    /// Power the host off. `params`: none.
    pub const SYSTEM_POWER_OFF: &str = "system.power_off";
    /// Power the host on (out-of-band). `params`: none.
    pub const SYSTEM_POWER_ON: &str = "system.power_on";
    /// Run a shell command. `params`: `command` (required), optional `cwd`.
    pub const COMMAND_RUN: &str = "command.run";
    /// Ensure a package is present/absent. `params`: `name`, `state`
    /// (`present`|`absent`), optional `version`.
    pub const PACKAGE_ENSURE: &str = "package.ensure";
    /// Ensure a service's run/enable state. `params`: `name`, `state`
    /// (`started`|`stopped`), `enabled` (`true`|`false`).
    pub const SERVICE_ENSURE: &str = "service.ensure";
    /// Write a file idempotently. `params`: `path`, `content`, optional `mode`
    /// (octal, e.g. `0644`), `owner`, `group`.
    pub const FILE_WRITE: &str = "file.write";
    /// Ensure a role's packages/services are present. `params`: `role`.
    /// Used only as a fallback for roles without a built-in expansion.
    pub const ROLE_PROVISION: &str = "role.provision";
    /// Fallback passthrough for an intent the planner doesn't specialize.
    /// `params`: the intent's params verbatim, plus `intent`.
    pub const INTENT_CUSTOM: &str = "intent.custom";
}

/// What Nexus forwards to the orchestrator, mirroring Daedalus IT's TaskRequest.
#[derive(Debug, Clone, Deserialize)]
pub struct PlanRequest {
    pub intent: String,
    #[serde(default)]
    pub targets: Vec<String>,
    #[serde(default, alias = "requesterId")]
    pub requester_id: String,
    #[serde(default, alias = "autoRollback")]
    pub auto_rollback: bool,
    #[serde(default)]
    pub params: BTreeMap<String, String>,
    /// Optional caller-supplied task id, so Nexus and the orchestrator agree on
    /// one correlation id. Generated if absent.
    #[serde(default, alias = "taskId")]
    pub task_id: Option<String>,
}

/// One executable step. `compensation`, when present, is the action an agent
/// runs to undo this step during rollback.
#[derive(Debug, Clone, Serialize)]
pub struct PlanStep {
    pub id: String,
    pub action: String,
    pub params: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compensation: Option<Compensation>,
    /// Whether this step, if it fails, should abort the whole plan.
    pub critical: bool,
}

/// A minimal inverse action for rollback.
#[derive(Debug, Clone, Serialize)]
pub struct Compensation {
    pub action: String,
    pub params: BTreeMap<String, String>,
}

/// An ordered, targeted plan derived from one intent.
#[derive(Debug, Clone, Serialize)]
pub struct TransactionPlan {
    pub task_id: String,
    pub intent: String,
    pub targets: Vec<String>,
    pub auto_rollback: bool,
    pub steps: Vec<PlanStep>,
}

/// The orchestrator's answer to a plan request.
#[derive(Debug, Clone, Serialize)]
pub struct PlanResponse {
    pub task_id: String,
    pub status: String,
    pub plan: TransactionPlan,
}

fn step(action: &str, params: BTreeMap<String, String>, critical: bool) -> PlanStep {
    PlanStep {
        id: Uuid::now_v7().to_string(),
        action: action.to_string(),
        params,
        compensation: None,
        critical,
    }
}

/// Build a params map from key/value pairs, skipping empty values.
fn params_of(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

fn get<'a>(p: &'a BTreeMap<String, String>, key: &str) -> &'a str {
    p.get(key).map_or("", String::as_str)
}

/// A `package.ensure` step, with a compensation that removes what it installed.
fn package_step(name: &str, state: &str, version: &str) -> PlanStep {
    let mut s = step(
        action::PACKAGE_ENSURE,
        params_of(&[("name", name), ("state", state), ("version", version)]),
        true,
    );
    // Installing is reversible by removing; removing is not auto-compensated.
    if state == "present" {
        s.compensation = Some(Compensation {
            action: action::PACKAGE_ENSURE.to_string(),
            params: params_of(&[("name", name), ("state", "absent")]),
        });
    }
    s
}

/// A `service.ensure` step.
fn service_step(name: &str, run_state: &str, enabled: &str) -> PlanStep {
    step(
        action::SERVICE_ENSURE,
        params_of(&[("name", name), ("state", run_state), ("enabled", enabled)]),
        true,
    )
}

/// Steps to provision a known role. Unknown roles fall back to a single
/// `role.provision` marker step so nothing is silently dropped.
fn provision_steps(role: &str) -> Vec<PlanStep> {
    match role {
        "webserver" => vec![
            package_step("nginx", "present", ""),
            service_step("nginx", "started", "true"),
        ],
        "database" => vec![
            package_step("postgresql", "present", ""),
            service_step("postgresql", "started", "true"),
        ],
        _ => vec![step(
            action::ROLE_PROVISION,
            params_of(&[("role", role)]),
            true,
        )],
    }
}

/// Resolve an intent into a concrete plan.
///
/// Recognized intents mirror the node actions Daedalus IT dispatches
/// (`restart_node`, `power_off_node`, `power_on_node`, `run_command`) plus the
/// `provision_<role>` family from client onboarding. Anything else is planned
/// as a single passthrough `intent.custom` step so unknown intents degrade
/// safely rather than being dropped.
pub fn plan_intent(req: &PlanRequest) -> TransactionPlan {
    let task_id = req
        .task_id
        .clone()
        .unwrap_or_else(|| Uuid::now_v7().to_string());

    let steps = match req.intent.as_str() {
        "restart_node" => vec![step(action::SYSTEM_REBOOT, BTreeMap::new(), true)],
        "power_off_node" => vec![step(action::SYSTEM_POWER_OFF, BTreeMap::new(), true)],
        "power_on_node" => vec![step(action::SYSTEM_POWER_ON, BTreeMap::new(), true)],
        "run_command" => {
            let mut p = BTreeMap::new();
            if let Some(cmd) = req.params.get("command") {
                p.insert("command".to_string(), cmd.clone());
            }
            if let Some(cwd) = req.params.get("cwd") {
                p.insert("cwd".to_string(), cwd.clone());
            }
            vec![step(action::COMMAND_RUN, p, true)]
        }
        "install_package" => {
            let name = get(&req.params, "name");
            let name = if name.is_empty() {
                get(&req.params, "package")
            } else {
                name
            };
            vec![package_step(name, "present", get(&req.params, "version"))]
        }
        "remove_package" => {
            let name = get(&req.params, "name");
            let name = if name.is_empty() {
                get(&req.params, "package")
            } else {
                name
            };
            vec![package_step(name, "absent", "")]
        }
        "manage_service" => {
            let enabled = get(&req.params, "enabled");
            let enabled = if enabled.is_empty() { "true" } else { enabled };
            let run_state = get(&req.params, "state");
            let run_state = if run_state.is_empty() {
                "started"
            } else {
                run_state
            };
            vec![service_step(get(&req.params, "name"), run_state, enabled)]
        }
        "deploy_file" => vec![step(
            action::FILE_WRITE,
            params_of(&[
                ("path", get(&req.params, "path")),
                ("content", get(&req.params, "content")),
                ("mode", get(&req.params, "mode")),
                ("owner", get(&req.params, "owner")),
                ("group", get(&req.params, "group")),
            ]),
            true,
        )],
        other if other.starts_with("provision_") => {
            provision_steps(other.trim_start_matches("provision_"))
        }
        _ => {
            let mut p = req.params.clone();
            p.insert("intent".to_string(), req.intent.clone());
            vec![step(action::INTENT_CUSTOM, p, false)]
        }
    };

    TransactionPlan {
        task_id,
        intent: req.intent.clone(),
        targets: req.targets.clone(),
        auto_rollback: req.auto_rollback,
        steps,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(intent: &str) -> PlanRequest {
        PlanRequest {
            intent: intent.to_string(),
            targets: vec!["agent-1".into()],
            requester_id: "user:alice".into(),
            auto_rollback: true,
            params: BTreeMap::new(),
            task_id: None,
        }
    }

    #[test]
    fn restart_maps_to_reboot() {
        let plan = plan_intent(&req("restart_node"));
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].action, action::SYSTEM_REBOOT);
        assert!(plan.steps[0].critical);
        assert!(!plan.task_id.is_empty());
    }

    #[test]
    fn run_command_carries_command_param() {
        let mut r = req("run_command");
        r.params.insert("command".into(), "systemctl status nginx".into());
        let plan = plan_intent(&r);
        assert_eq!(plan.steps[0].action, action::COMMAND_RUN);
        assert_eq!(
            plan.steps[0].params.get("command").map(String::as_str),
            Some("systemctl status nginx")
        );
    }

    #[test]
    fn install_package_maps_to_package_ensure() {
        let mut r = req("install_package");
        r.params.insert("name".into(), "nginx".into());
        r.params.insert("version".into(), "1.24".into());
        let plan = plan_intent(&r);
        assert_eq!(plan.steps[0].action, action::PACKAGE_ENSURE);
        assert_eq!(
            plan.steps[0].params.get("state").map(String::as_str),
            Some("present")
        );
        assert_eq!(
            plan.steps[0].params.get("version").map(String::as_str),
            Some("1.24")
        );
        // Installing carries a remove compensation.
        assert!(plan.steps[0].compensation.is_some());
    }

    #[test]
    fn manage_service_defaults_started_enabled() {
        let mut r = req("manage_service");
        r.params.insert("name".into(), "nginx".into());
        let plan = plan_intent(&r);
        assert_eq!(plan.steps[0].action, action::SERVICE_ENSURE);
        assert_eq!(
            plan.steps[0].params.get("state").map(String::as_str),
            Some("started")
        );
        assert_eq!(
            plan.steps[0].params.get("enabled").map(String::as_str),
            Some("true")
        );
    }

    #[test]
    fn deploy_file_carries_path_and_content() {
        let mut r = req("deploy_file");
        r.params.insert("path".into(), "/etc/motd".into());
        r.params.insert("content".into(), "hello".into());
        r.params.insert("mode".into(), "0644".into());
        let plan = plan_intent(&r);
        assert_eq!(plan.steps[0].action, action::FILE_WRITE);
        assert_eq!(
            plan.steps[0].params.get("path").map(String::as_str),
            Some("/etc/motd")
        );
    }

    #[test]
    fn provision_webserver_expands_to_package_and_service() {
        let plan = plan_intent(&req("provision_webserver"));
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.steps[0].action, action::PACKAGE_ENSURE);
        assert_eq!(
            plan.steps[0].params.get("name").map(String::as_str),
            Some("nginx")
        );
        assert_eq!(plan.steps[1].action, action::SERVICE_ENSURE);
    }

    #[test]
    fn provision_unknown_role_falls_back_to_stub() {
        let plan = plan_intent(&req("provision_mystery"));
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].action, action::ROLE_PROVISION);
        assert_eq!(
            plan.steps[0].params.get("role").map(String::as_str),
            Some("mystery")
        );
    }

    #[test]
    fn unknown_intent_degrades_to_custom() {
        let plan = plan_intent(&req("do_something_novel"));
        assert_eq!(plan.steps[0].action, action::INTENT_CUSTOM);
        assert_eq!(
            plan.steps[0].params.get("intent").map(String::as_str),
            Some("do_something_novel")
        );
        assert!(!plan.steps[0].critical);
    }

    #[test]
    fn caller_task_id_is_preserved() {
        let mut r = req("restart_node");
        r.task_id = Some("fixed-task-id".into());
        let plan = plan_intent(&r);
        assert_eq!(plan.task_id, "fixed-task-id");
    }
}
