# linexus-orch
Orchestrator for the Linexus ecosystem — Demand/Supply engine, DAG planner, labor routing, 2-year KPI rebalancing. The nervous system that matches existence to economy.

## The RMM planner

`src/plan.rs` is the operational half: Nexus forwards an intent, the planner
resolves it into an ordered `TransactionPlan` of canonical steps the agent's
executor matches on. The step vocabulary is the contract, so it stays small and
explicit. An intent the planner does not know is **rejected** — `POST /plan`
answers `400 {"error": "unknown intent \"…\"", "intent": "…"}` and Nexus fails
the task with that reason — rather than passed through as a step the agent
cannot run. (Earlier versions planned an `intent.custom` passthrough, which the
agent silently skipped and reported as a success.)

| Intent | Steps |
| --- | --- |
| `restart_node` / `power_off_node` / `power_on_node` | `system.reboot` / `system.power_off` / `system.power_on` |
| `run_command` | `command.run` |
| `install_package` / `remove_package` | `package.ensure` (installing carries a remove compensation) |
| `manage_service` | `service.ensure` |
| `deploy_file` | `file.write` |
| `provision_<role>` | the role's packages + services, or a `role.provision` marker |
| `set_environment` | `agent.environment` |
| `restart_service` / `start_service` / `stop_service` | `service.ensure` with `restart: "true"` / `state: started` / `state: stopped` (the unit comes from param `service`, or `name`) |
| `refresh_inventory` | `agent.facts` — the agent ships a facts report now |
| `lock_session` | `command.run` `loginctl lock-sessions` |
| `install_dns_server` | `dns.server.ensure` (critical; no params) |
| `dns_zone_apply` | `dns.zone.apply` (critical): `zone`, `content`, `role` (default `primary`), `primaries`, `secondaries`, `serial` |
| `dns_zone_remove` | `dns.zone.remove` (critical): `zone` |
| `mount_volume` | `disk.mount` (critical): `device`, `mountPoint`, `fsType` (default `ext4`), `format` (default `if_blank`) |
| `run_script` | `script.run` (critical, no compensation): `script` (required — missing is a `400`), `interpreter` (from `interpreter`, else the Hub's `shell` keyword, default `/bin/sh`), `timeout_seconds` (from `timeout_seconds`, else `timeout`, default `600`), `args`, `elevated` |
| `list_patches` | `patches.list` (read-only; the agent writes JSON to the step output) |
| `install_patches` | `patches.install` (critical, no compensation): `packages` (from `packages`, else `ids`, default `all`) — the agent runs it only with its destructive allowance |
| `list_vms` | `vms.list` (read-only; JSON in the step output) |

Params that are absent or empty on the intent are left out of the step, never
sent as empty strings. The infrastructure intents (`install_dns_server`,
`dns_zone_*`, `mount_volume`) are what Nexus dispatches for its DNS and cloud
APIs; the service, inventory and lock intents are what the Hub's machine
command panel sends.

### `set_environment`

Sets which environment a machine belongs to and whether it is tracked —
params `environment`, `monitored`, optional `note`. It is the one intent whose
step is **not critical and not compensated**: failing to apply a label should
not abort a batch, and "undoing" it would mean guessing what the machine was
before, which both the Hub and Nexus already know. A re-push is the honest
repair.

Both params degrade toward the loud option. An empty environment plans as
`production`, and anything that is not an explicit `false`/`0`/`no` plans as
tracked — a malformed parameter must never be the reason a production box goes
quiet.

## Running

| Variable | Default | Meaning |
| --- | --- | --- |
| `ORCH_BIND` | `0.0.0.0:5152` | Listen address |
| `ORCH_SERVICE_TOKEN` | unset | Bearer token Nexus presents on `POST /plan`. Unset means open (development only) |
| `LINEXUS_ENV` | unset | `production` marks a real deployment: the service **refuses to start** without `ORCH_SERVICE_TOKEN` instead of warning |

`GET /healthz` is liveness; `GET /metrics` serves Prometheus counters
(`orch_http_requests_total`, `orch_plans_total`, `orch_plan_failures_total`).
Both are unauthenticated.
