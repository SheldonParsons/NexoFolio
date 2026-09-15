fn main() {
    use nexofolio_contracts::*;
    println!(
        "{}",
        serde_json::json!({
         "checkpoints.schema.json":schemars::schema_for!(MaintenanceCheckpointPage),"snapshot.schema.json":schemars::schema_for!(KnowledgeSnapshot),"reply.schema.json":schemars::schema_for!(MaintenanceReply),"plan.schema.json":schemars::schema_for!(MaintenancePlan),"run.schema.json":schemars::schema_for!(MaintenanceRun),"page.schema.json":schemars::schema_for!(MaintenancePage),"candidate.schema.json":schemars::schema_for!(MaintenanceCandidate),"start.schema.json":schemars::schema_for!(StartMaintenance),"publish.schema.json":schemars::schema_for!(PublishKnowledge),"restore.schema.json":schemars::schema_for!(RestoreKnowledge),"activation.schema.json":schemars::schema_for!(KnowledgeActivation),"versions.schema.json":schemars::schema_for!(KnowledgeVersionPage),"interface-knowledge.schema.json":schemars::schema_for!(InterfaceKnowledge)
        })
    );
}
