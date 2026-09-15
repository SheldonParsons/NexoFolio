use clap::{Parser, Subcommand};
use nexofolio_backend::wiring::{Config, init_logging};
use nexofolio_infrastructure::Postgres;
use nexofolio_intake::ProjectPathPolicies;

#[derive(Parser)]
#[command(about = "NexoFolio foundation administration")]
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
    CatalogCreate {
        #[arg(long)]
        project_id: nexofolio_contracts::ProjectId,
    },
    CatalogRun {
        #[arg(long)]
        task_id: nexofolio_contracts::JobId,
    },
    CatalogShow {
        #[arg(long)]
        task_id: nexofolio_contracts::JobId,
    },
    CatalogPreview {
        #[arg(long)]
        project_id: nexofolio_contracts::ProjectId,
    },
    Migrate,
    ProcessOne,
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
        Command::CatalogCreate { project_id } => {
            use nexofolio_knowledge::CatalogPreviewStore;
            let task = nexofolio_infrastructure::PostgresCatalogPreviews::new(database.clone())
                .create(project_id)
                .await?;
            println!(
                "{}",
                serde_json::json!({"task_id":task.task_id,"candidate_id":task.candidate_id,"status":task.status,"snapshot_interfaces":task.snapshot.interfaces.len(),"snapshot_sha256":task.snapshot_sha256})
            );
        }
        Command::CatalogShow { task_id } => {
            use nexofolio_knowledge::CatalogPreviewStore;
            let task = nexofolio_infrastructure::PostgresCatalogPreviews::new(database.clone())
                .read(task_id)
                .await?;
            println!("{}", serde_json::to_string_pretty(&task)?);
        }
        Command::CatalogRun { task_id } => {
            let task =
                nexofolio_backend::wiring::build_catalog_previews(&config, database.clone())?
                    .run(task_id)
                    .await?;
            println!(
                "{}",
                serde_json::json!({"task_id":task.task_id,"candidate_id":task.candidate_id,"status":task.status,"review":task.review})
            );
        }
        Command::CatalogPreview { project_id } => {
            let service =
                nexofolio_backend::wiring::build_catalog_previews(&config, database.clone())?;
            let task = service.create(project_id).await?;
            eprintln!("Catalog preview task: {}", task.task_id);
            let task = service.run(task.task_id).await?;
            println!(
                "{}",
                serde_json::json!({"task_id":task.task_id,"candidate_id":task.candidate_id,"status":task.status,"review":task.review})
            );
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
