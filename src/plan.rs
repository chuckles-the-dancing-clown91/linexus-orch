//! Transaction planning for infrastructure (RMM) intents.
//!
//! Daedalus IT submits high-level intents through Nexus — "restart this node",
//! "run this command", "provision this role". The orchestrator resolves each
//! intent into an ordered [`TransactionPlan`] of canonical [`PlanStep`]s that an
//! agent knows how to execute. The step vocabulary here is the contract the
//! agent implements, so it is deliberately small and explicit — and an intent
//! outside it is a [`PlanError`], never a plan the agent cannot run.
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
    /// (`started`|`stopped`), `enabled` (`true`|`false`), `restart`
    /// (`true` restarts the service unconditionally — the one verb that is
    /// not idempotent, and always reports changed).
    pub const SERVICE_ENSURE: &str = "service.ensure";
    /// Write a file idempotently. `params`: `path`, `content`, optional `mode`
    /// (octal, e.g. `0644`), `owner`, `group`.
    pub const FILE_WRITE: &str = "file.write";
    /// Ensure a role's packages/services are present. `params`: `role`.
    /// Used only as a fallback for roles without a built-in expansion.
    pub const ROLE_PROVISION: &str = "role.provision";
    /// Set the agent's environment and whether it is tracked. `params`:
    /// `environment` (required), `monitored` (`true`|`false`), optional
    /// `note`. The agent persists both locally and stops shipping telemetry
    /// while untracked — an agent that doesn't know it is untracked keeps
    /// reporting, and the silence the operator asked for never arrives.
    pub const AGENT_ENVIRONMENT: &str = "agent.environment";
    /// Make the agent collect and ship a facts report now (it is sent even
    /// while the agent is not monitored — an operator asked). `params`: none.
    pub const AGENT_FACTS: &str = "agent.facts";
    /// Install and start a BIND server and prepare the agent-managed include
    /// file and zones directory. `params`: none — the agent picks `bind9`
    /// on apt and `bind` on dnf.
    pub const DNS_SERVER_ENSURE: &str = "dns.server.ensure";
    /// Serve a zone. `params`: `zone`, `content` (the whole zone file;
    /// primaries only), `role` (`primary`|`secondary`), `primaries`
    /// (comma-separated IPs; secondaries only), `secondaries`
    /// (comma-separated IPs a primary allows transfers to and notifies),
    /// `serial`. The agent validates the file with `named-checkzone` before
    /// swapping it in, and reloads only on change.
    pub const DNS_ZONE_APPLY: &str = "dns.zone.apply";
    /// Stop serving a zone and delete its file. `params`: `zone`.
    pub const DNS_ZONE_REMOVE: &str = "dns.zone.remove";
    /// Mount a block device. `params`: `device`, `mountPoint`, `fsType`
    /// (`ext4`|`xfs`), `format` (`if_blank`|`never`). The agent never
    /// formats a device that already carries a filesystem or partition
    /// table, writes an fstab line by UUID with `nofail`, and mounts.
    pub const DISK_MOUNT: &str = "disk.mount";
    /// Run a script. `params`: `script` (required), `interpreter` (a path or
    /// program; default `/bin/sh`), `timeout_seconds` (default `600`),
    /// optional `args` (whitespace-separated), `elevated` (`true` runs as
    /// root). The agent writes the script to a private temp file, runs it
    /// and reports the exit code.
    pub const SCRIPT_RUN: &str = "script.run";
    /// List pending package upgrades (read-only). `params`: none; the agent
    /// writes `{manager, count, patches: [...]}` JSON to the step output.
    pub const PATCHES_LIST: &str = "patches.list";
    /// Install package upgrades. `params`: `packages` — comma-separated
    /// names or `name=version` pairs, or `all`. The agent treats this as
    /// destructive (suppressed without its destructive allowance).
    pub const PATCHES_INSTALL: &str = "patches.install";
    /// List virtual machines on a hypervisor host (read-only). `params`:
    /// none; the agent writes `{hypervisor, count, vms: [...]}` JSON.
    pub const VMS_LIST: &str = "vms.list";
}

/// Why an intent could not be planned. Returned to Nexus as `400` so the
/// task fails at once with the reason, instead of reaching an agent as a
/// step it cannot execute.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlanError {
    #[error("unknown intent {0:?}")]
    UnknownIntent(String),
    #[error("intent {intent:?} needs param {param:?}")]
    MissingParam { intent: String, param: &'static str },
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

/// The service an intent names: the Hub sends `service`, older callers `name`.
fn service_param(p: &BTreeMap<String, String>) -> &str {
    let service = get(p, "service");
    if service.is_empty() {
        get(p, "name")
    } else {
        service
    }
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

/// The value of `key` in `p`, or `default` when absent or empty.
fn get_or<'a>(p: &'a BTreeMap<String, String>, key: &str, default: &'a str) -> &'a str {
    let v = get(p, key);
    if v.is_empty() { default } else { v }
}

/// Resolve an intent into a concrete plan.
///
/// Recognized intents mirror the node actions Daedalus IT dispatches
/// (`restart_node`, `power_off_node`, `power_on_node`, `run_command`,
/// `lock_session`, `refresh_inventory`, `restart_service` / `start_service` /
/// `stop_service`), its automation and patching intents (`run_script`,
/// `list_patches`, `install_patches`, `list_vms`), the infrastructure
/// intents Nexus dispatches (`install_dns_server`, `dns_zone_apply`,
/// `dns_zone_remove`, `mount_volume`), plus the `provision_<role>` family
/// from client onboarding. Anything else is a [`PlanError::UnknownIntent`]:
/// the agent has no step for it, and a loud planning failure beats a plan
/// that reaches the host and fails there (or, worse, is skipped).
pub fn plan_intent(req: &PlanRequest) -> Result<TransactionPlan, PlanError> {
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
        "set_environment" => {
            let env = get(&req.params, "environment");
            let env = if env.is_empty() { "production" } else { env };
            // Anything other than an explicit false leaves the machine
            // tracked: a malformed param must not be the reason a production
            // box goes quiet.
            let monitored = match get(&req.params, "monitored").to_ascii_lowercase().as_str() {
                "false" | "0" | "no" => "false",
                _ => "true",
            };
            // Not critical, and not compensated. Failing to apply an
            // environment label should not abort a batch, and "undoing" it
            // would mean guessing what the machine was before — the Hub and
            // Nexus both hold that answer, so a re-push is the honest repair.
            vec![step(
                action::AGENT_ENVIRONMENT,
                params_of(&[
                    ("environment", env),
                    ("monitored", monitored),
                    ("note", get(&req.params, "note")),
                ]),
                false,
            )]
        }
        "restart_service" => vec![step(
            action::SERVICE_ENSURE,
            params_of(&[("name", service_param(&req.params)), ("restart", "true")]),
            true,
        )],
        "start_service" => vec![step(
            action::SERVICE_ENSURE,
            params_of(&[("name", service_param(&req.params)), ("state", "started")]),
            true,
        )],
        "stop_service" => vec![step(
            action::SERVICE_ENSURE,
            params_of(&[("name", service_param(&req.params)), ("state", "stopped")]),
            true,
        )],
        "refresh_inventory" => vec![step(action::AGENT_FACTS, BTreeMap::new(), true)],
        // Locks every graphical session on the machine; a server with none
        // succeeds without doing anything.
        "lock_session" => vec![step(
            action::COMMAND_RUN,
            params_of(&[("command", "loginctl lock-sessions")]),
            true,
        )],
        "install_dns_server" => vec![step(action::DNS_SERVER_ENSURE, BTreeMap::new(), true)],
        "dns_zone_apply" => {
            let role = get(&req.params, "role");
            let role = if role.is_empty() { "primary" } else { role };
            vec![step(
                action::DNS_ZONE_APPLY,
                params_of(&[
                    ("zone", get(&req.params, "zone")),
                    ("content", get(&req.params, "content")),
                    ("role", role),
                    ("primaries", get(&req.params, "primaries")),
                    ("secondaries", get(&req.params, "secondaries")),
                    ("serial", get(&req.params, "serial")),
                ]),
                true,
            )]
        }
        "dns_zone_remove" => vec![step(
            action::DNS_ZONE_REMOVE,
            params_of(&[("zone", get(&req.params, "zone"))]),
            true,
        )],
        "mount_volume" => {
            let fs_type = get(&req.params, "fsType");
            let fs_type = if fs_type.is_empty() { "ext4" } else { fs_type };
            let format = get(&req.params, "format");
            let format = if format.is_empty() {
                "if_blank"
            } else {
                format
            };
            vec![step(
                action::DISK_MOUNT,
                params_of(&[
                    ("device", get(&req.params, "device")),
                    ("mountPoint", get(&req.params, "mountPoint")),
                    ("fsType", fs_type),
                    ("format", format),
                ]),
                true,
            )]
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
        "run_script" => {
            let script = get(&req.params, "script");
            if script.is_empty() {
                return Err(PlanError::MissingParam {
                    intent: req.intent.clone(),
                    param: "script",
                });
            }
            // The Hub names the interpreter by shell keyword (`shell`:
            // bash/python/powershell) and the timeout as `timeout`; an
            // explicit `interpreter` / `timeout_seconds` wins.
            let interpreter = get_or(
                &req.params,
                "interpreter",
                get_or(&req.params, "shell", "/bin/sh"),
            );
            let timeout = get_or(
                &req.params,
                "timeout_seconds",
                get_or(&req.params, "timeout", "600"),
            );
            vec![step(
                action::SCRIPT_RUN,
                params_of(&[
                    ("interpreter", interpreter),
                    ("script", script),
                    ("timeout_seconds", timeout),
                    ("args", get(&req.params, "args")),
                    ("elevated", get(&req.params, "elevated")),
                ]),
                true,
            )]
        }
        "list_patches" => vec![step(action::PATCHES_LIST, BTreeMap::new(), true)],
        "install_patches" => {
            // The Hub sends `packages` (comma-separated `name=version`);
            // `ids` is accepted for hand-written intents. Empty means all.
            let packages = get_or(&req.params, "packages", get_or(&req.params, "ids", "all"));
            // Not compensated: there is no safe generic "downgrade", and the
            // agent already refuses to run this without its destructive
            // allowance.
            vec![step(
                action::PATCHES_INSTALL,
                params_of(&[("packages", packages)]),
                true,
            )]
        }
        "list_vms" => vec![step(action::VMS_LIST, BTreeMap::new(), true)],
        other if other.starts_with("provision_") => {
            provision_steps(other.trim_start_matches("provision_"))
        }
        other => return Err(PlanError::UnknownIntent(other.to_string())),
    };

    Ok(TransactionPlan {
        task_id,
        intent: req.intent.clone(),
        targets: req.targets.clone(),
        auto_rollback: req.auto_rollback,
        steps,
    })
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

    /// Plan an intent the tests expect to be plannable.
    fn plan_intent(req: &PlanRequest) -> TransactionPlan {
        super::plan_intent(req).unwrap_or_else(|e| panic!("{}: {e}", req.intent))
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
        r.params
            .insert("command".into(), "systemctl status nginx".into());
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
    fn set_environment_carries_environment_and_tracking() {
        let mut r = req("set_environment");
        r.params.insert("environment".into(), "development".into());
        r.params.insert("monitored".into(), "false".into());
        r.params.insert("note".into(), "lab rebuild".into());
        let plan = plan_intent(&r);
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].action, action::AGENT_ENVIRONMENT);
        assert_eq!(
            plan.steps[0].params.get("environment").map(String::as_str),
            Some("development")
        );
        assert_eq!(
            plan.steps[0].params.get("monitored").map(String::as_str),
            Some("false")
        );
        assert_eq!(
            plan.steps[0].params.get("note").map(String::as_str),
            Some("lab rebuild")
        );
        // A label is not worth aborting a batch over.
        assert!(!plan.steps[0].critical);
    }

    #[test]
    fn set_environment_defaults_to_tracked_production() {
        // A caller that sends neither param must not mute the machine, and
        // must not leave it with an empty environment nothing can group by.
        let plan = plan_intent(&req("set_environment"));
        assert_eq!(
            plan.steps[0].params.get("environment").map(String::as_str),
            Some("production")
        );
        assert_eq!(
            plan.steps[0].params.get("monitored").map(String::as_str),
            Some("true")
        );
    }

    #[test]
    fn set_environment_treats_garbage_tracking_as_tracked() {
        let mut r = req("set_environment");
        r.params.insert("monitored".into(), "maybe".into());
        let plan = plan_intent(&r);
        assert_eq!(
            plan.steps[0].params.get("monitored").map(String::as_str),
            Some("true")
        );
    }

    #[test]
    fn unknown_intent_fails_planning() {
        assert_eq!(
            super::plan_intent(&req("do_something_novel")).unwrap_err(),
            PlanError::UnknownIntent("do_something_novel".into())
        );
        assert_eq!(
            PlanError::UnknownIntent("do_something_novel".into()).to_string(),
            "unknown intent \"do_something_novel\""
        );
    }

    #[test]
    fn run_script_maps_to_script_run_with_defaults() {
        let mut r = req("run_script");
        r.params.insert("script".into(), "echo hi".into());
        let plan = plan_intent(&r);
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].action, action::SCRIPT_RUN);
        assert!(plan.steps[0].critical);
        assert!(plan.steps[0].compensation.is_none());
        assert_eq!(param(&plan, "script"), Some("echo hi"));
        assert_eq!(param(&plan, "interpreter"), Some("/bin/sh"));
        assert_eq!(param(&plan, "timeout_seconds"), Some("600"));
        assert_eq!(param(&plan, "args"), None);

        // What the Hub's automation runner sends.
        r.params.insert("shell".into(), "bash".into());
        r.params.insert("timeout".into(), "300".into());
        r.params.insert("args".into(), "--verbose".into());
        r.params.insert("elevated".into(), "true".into());
        let plan = plan_intent(&r);
        assert_eq!(param(&plan, "interpreter"), Some("bash"));
        assert_eq!(param(&plan, "timeout_seconds"), Some("300"));
        assert_eq!(param(&plan, "args"), Some("--verbose"));
        assert_eq!(param(&plan, "elevated"), Some("true"));

        // Explicit names win over the Hub's.
        r.params
            .insert("interpreter".into(), "/usr/bin/perl".into());
        r.params.insert("timeout_seconds".into(), "42".into());
        let plan = plan_intent(&r);
        assert_eq!(param(&plan, "interpreter"), Some("/usr/bin/perl"));
        assert_eq!(param(&plan, "timeout_seconds"), Some("42"));
    }

    #[test]
    fn run_script_without_a_script_fails_planning() {
        assert_eq!(
            super::plan_intent(&req("run_script")).unwrap_err(),
            PlanError::MissingParam {
                intent: "run_script".into(),
                param: "script",
            }
        );
    }

    #[test]
    fn patch_intents_map_to_patch_steps() {
        let plan = plan_intent(&req("list_patches"));
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].action, action::PATCHES_LIST);
        assert!(plan.steps[0].params.is_empty());

        let plan = plan_intent(&req("install_patches"));
        assert_eq!(plan.steps[0].action, action::PATCHES_INSTALL);
        assert!(plan.steps[0].critical);
        assert!(plan.steps[0].compensation.is_none());
        assert_eq!(param(&plan, "packages"), Some("all"));

        let mut r = req("install_patches");
        r.params
            .insert("packages".into(), "nginx=1.24-1,openssl".into());
        assert_eq!(
            param(&plan_intent(&r), "packages"),
            Some("nginx=1.24-1,openssl")
        );
        let mut r = req("install_patches");
        r.params.insert("ids".into(), "nginx".into());
        assert_eq!(param(&plan_intent(&r), "packages"), Some("nginx"));
    }

    #[test]
    fn list_vms_maps_to_vms_list() {
        let plan = plan_intent(&req("list_vms"));
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].action, action::VMS_LIST);
        assert!(plan.steps[0].params.is_empty());
    }

    fn param<'a>(plan: &'a TransactionPlan, key: &str) -> Option<&'a str> {
        plan.steps[0].params.get(key).map(String::as_str)
    }

    #[test]
    fn service_intents_map_to_service_ensure() {
        for (intent, key, value) in [
            ("restart_service", "restart", "true"),
            ("start_service", "state", "started"),
            ("stop_service", "state", "stopped"),
        ] {
            let mut r = req(intent);
            // The Hub sends the unit as `service`.
            r.params.insert("service".into(), "nginx.service".into());
            let plan = plan_intent(&r);
            assert_eq!(plan.steps.len(), 1, "{intent}");
            assert_eq!(plan.steps[0].action, action::SERVICE_ENSURE, "{intent}");
            assert_eq!(param(&plan, "name"), Some("nginx.service"), "{intent}");
            assert_eq!(param(&plan, key), Some(value), "{intent}");
            // Start/stop/restart never touch the enable state.
            assert_eq!(param(&plan, "enabled"), None, "{intent}");
            assert!(plan.steps[0].critical, "{intent}");
        }
        let mut r = req("restart_service");
        r.params.insert("name".into(), "sshd".into());
        assert_eq!(param(&plan_intent(&r), "name"), Some("sshd"));
        assert_eq!(param(&plan_intent(&r), "state"), None);
    }

    #[test]
    fn refresh_inventory_maps_to_agent_facts() {
        let plan = plan_intent(&req("refresh_inventory"));
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].action, action::AGENT_FACTS);
        assert!(plan.steps[0].params.is_empty());
    }

    #[test]
    fn lock_session_runs_loginctl() {
        let plan = plan_intent(&req("lock_session"));
        assert_eq!(plan.steps[0].action, action::COMMAND_RUN);
        assert_eq!(param(&plan, "command"), Some("loginctl lock-sessions"));
    }

    #[test]
    fn install_dns_server_maps_to_dns_server_ensure() {
        let plan = plan_intent(&req("install_dns_server"));
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].action, action::DNS_SERVER_ENSURE);
        assert!(plan.steps[0].params.is_empty());
        assert!(plan.steps[0].critical);
    }

    #[test]
    fn dns_zone_apply_carries_every_param() {
        let mut r = req("dns_zone_apply");
        for (k, v) in [
            ("zone", "example.com"),
            ("content", "$TTL 3600\n@ IN SOA ns1 hostmaster 1 2 3 4 5\n"),
            ("role", "secondary"),
            ("primaries", "10.0.0.1,10.0.0.2"),
            ("secondaries", "10.0.0.9"),
            ("serial", "2026100901"),
            ("unrelated", "dropped"),
        ] {
            r.params.insert(k.into(), v.into());
        }
        let plan = plan_intent(&r);
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].action, action::DNS_ZONE_APPLY);
        assert!(plan.steps[0].critical);
        assert_eq!(param(&plan, "zone"), Some("example.com"));
        assert_eq!(
            param(&plan, "content"),
            Some("$TTL 3600\n@ IN SOA ns1 hostmaster 1 2 3 4 5\n")
        );
        assert_eq!(param(&plan, "role"), Some("secondary"));
        assert_eq!(param(&plan, "primaries"), Some("10.0.0.1,10.0.0.2"));
        assert_eq!(param(&plan, "secondaries"), Some("10.0.0.9"));
        assert_eq!(param(&plan, "serial"), Some("2026100901"));
        assert_eq!(param(&plan, "unrelated"), None);
    }

    #[test]
    fn dns_zone_apply_defaults_to_primary() {
        let mut r = req("dns_zone_apply");
        r.params.insert("zone".into(), "example.com".into());
        let plan = plan_intent(&r);
        assert_eq!(param(&plan, "role"), Some("primary"));
        // Absent params are absent, not empty strings.
        assert_eq!(param(&plan, "primaries"), None);
    }

    #[test]
    fn dns_zone_remove_carries_zone() {
        let mut r = req("dns_zone_remove");
        r.params.insert("zone".into(), "example.com".into());
        let plan = plan_intent(&r);
        assert_eq!(plan.steps[0].action, action::DNS_ZONE_REMOVE);
        assert_eq!(param(&plan, "zone"), Some("example.com"));
        assert_eq!(plan.steps[0].params.len(), 1);
    }

    #[test]
    fn mount_volume_maps_to_critical_disk_mount_with_defaults() {
        let mut r = req("mount_volume");
        r.params.insert(
            "device".into(),
            "/dev/disk/by-id/scsi-0DO_Volume_data".into(),
        );
        r.params.insert("mountPoint".into(), "/mnt/data".into());
        let plan = plan_intent(&r);
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.steps[0].action, action::DISK_MOUNT);
        assert!(plan.steps[0].critical);
        assert_eq!(
            param(&plan, "device"),
            Some("/dev/disk/by-id/scsi-0DO_Volume_data")
        );
        assert_eq!(param(&plan, "mountPoint"), Some("/mnt/data"));
        assert_eq!(param(&plan, "fsType"), Some("ext4"));
        assert_eq!(param(&plan, "format"), Some("if_blank"));

        r.params.insert("fsType".into(), "xfs".into());
        r.params.insert("format".into(), "never".into());
        let plan = plan_intent(&r);
        assert_eq!(param(&plan, "fsType"), Some("xfs"));
        assert_eq!(param(&plan, "format"), Some("never"));
    }

    #[test]
    fn caller_task_id_is_preserved() {
        let mut r = req("restart_node");
        r.task_id = Some("fixed-task-id".into());
        let plan = plan_intent(&r);
        assert_eq!(plan.task_id, "fixed-task-id");
    }
}
