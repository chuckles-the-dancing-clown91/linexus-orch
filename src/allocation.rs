//! Demand Allocation
use linexus_core::engine::{DemandEvent, OrchestratorEngine, TaskAssignment, TaskAssignmentStatus};
use linexus_core::errors::LinexusError;
use linexus_core::identity::NodeIdentity;
use uuid::Uuid;

pub struct AllocationService;

impl AllocationService {
    pub fn process_demand(demand: DemandEvent, available_nodes: &[NodeIdentity], current_time: u64) -> Result<TaskAssignment, LinexusError> {
        let candidate = OrchestratorEngine::route_demand(&demand, available_nodes)?;
        let status = match candidate {
            Some(node_id) => TaskAssignmentStatus::Offered { candidate_id: node_id },
            None => TaskAssignmentStatus::Unassigned,
        };
        Ok(TaskAssignment { task_id: Uuid::new_v4(), demand_event: demand, status, created_at: current_time })
    }
}
