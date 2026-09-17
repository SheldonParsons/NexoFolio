use clap::{Parser, Subcommand};
use nexofolio_backend::wiring::{Config, init_logging};
use nexofolio_infrastructure::Postgres;
use nexofolio_intake::ProjectPathPolicies;

#[derive(Parser)]
#[command(about = "NexoFolio administration; reconstruction uses the maintenance HTTP API")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    CheckConfig,
    PathPolicyShow {
        #[arg(long)]
        project_id: nexofolio_contracts::ProjectId,
    },
    PathPolicySet {
        #[arg(long)]
        project_id: nexofolio_contracts::ProjectId,
        #[arg(long)]
        disable: bool,
        #[arg(long)]
        keep_numbers: bool,
        #[arg(long)]
        literal_prefix: Vec<String>,
    },
    /// Inspect a historical directory candidate; this never generates one.
    CatalogShow {
        #[arg(long)]
        task_id: nexofolio_contracts::JobId,
    },
    Migrate,
    ProcessOne,
    /// Upgrade legacy evidence offline, keeping historical IDs and original recordings.
    RefreshEvidence {
        #[arg(long)]
        project_id: nexofolio_contracts::ProjectId,
    },
    RetryObservation {
        ingestion_id: uuid::Uuid,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let config = Config::from_env()?;
    init_logging(&config.log_filter)?;
    let database = Postgres::new(
        &config.database_url,
        config.database_max_connections,
        config.database_timeout,
    )?;
    let _access = nexofolio_backend::wiring::build_access(&config, database.clone())?;
    match cli.command {
        Command::PathPolicyShow { project_id } => {
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &nexofolio_infrastructure::PostgresPathPolicies::new(database.clone())
                        .read(project_id)
                        .await?
                )?
            );
        }
        Command::PathPolicySet {
            project_id,
            disable,
            keep_numbers,
            literal_prefix,
        } => {
            let policy = nexofolio_contracts::PathPolicy {
                enabled: !disable,
                numeric_segments: !keep_numbers,
                literal_prefixes: literal_prefix,
            };
            nexofolio_infrastructure::PostgresPathPolicies::new(database.clone())
                .set(project_id, &policy)
                .await?;
            println!("Path policy saved for future observations.");
        }
        Command::CatalogShow { task_id } => {
            let task = nexofolio_infrastructure::PostgresCatalogPreviews::new(database.clone())
                .read(task_id)
                .await?;
            println!("{}", serde_json::to_string_pretty(&task)?);
        }
        Command::RefreshEvidence { project_id } => {
            let store = nexofolio_infrastructure::PostgresCaptureStore::new(
                database.clone(),
                std::sync::Arc::new(nexofolio_infrastructure::FileBlobStore::new(
                    config.blob_root.clone(),
                )),
            );
            let queued = store.refresh_legacy_evidence(project_id).await?;
            let mut processed = 0;
            for _ in 0..queued {
                if !store.process_evidence_one().await? {
                    break;
                }
                processed += 1;
            }
            let remaining = store.refresh_legacy_evidence(project_id).await?;
            println!(
                "{}",
                serde_json::json!({"queued":queued,"processed":processed,"remaining":remaining})
            );
            if remaining != 0 {
                return Err(
                    "Evidence refresh incomplete; pending work is preserved for retry".into(),
                );
            }
        }
        Command::ProcessOne => {
            let service = nexofolio_application::ProcessingService::new(std::sync::Arc::new(
                nexofolio_infrastructure::PostgresDocuments::new(database.clone()),
            ));
            println!("{}", serde_json::to_string(&service.process_one().await?)?);
        }
        Command::RetryObservation { ingestion_id } => {
            use nexofolio_knowledge::ObservationProcessor;
            nexofolio_infrastructure::PostgresDocuments::new(database.clone())
                .retry_failed(ingestion_id)
                .await?;
            println!("Observation queued for retry.");
        }
        Command::CheckConfig => {
            println!("Configuration valid (database connectivity not checked).")
        }
        Command::Migrate => {
            database.migrate().await?;
            println!("Database migrations completed.");
        }
    }
    database.close().await;
    Ok(())
}
