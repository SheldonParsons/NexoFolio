use clap::{Parser, Subcommand};
use nexofolio_backend::wiring::{Config, Databases, build_api, init_logging, observe_report};
use nexofolio_common::ProjectId;
use nexofolio_observe::Observe;

#[derive(Parser)]
#[command(about = "NexoFolio foundation administration")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    CheckConfig,
    Migrate,
    /// What observe knows.
    Observe {
        #[command(subcommand)]
        command: ObserveCommand,
    },
}

#[derive(Subcommand)]
enum ObserveCommand {
    /// Endpoints, templates, field labels per environment and open decisions.
    Report {
        #[arg(long)]
        project: ProjectId,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let config = Config::from_env()?;
    init_logging(&config.log_filter)?;
    let databases = Databases::new(&config)?;
    let _api = build_api(&config, &databases)?;
    match cli.command {
        Command::CheckConfig => {
            println!("Configuration valid (database connectivity not checked).")
        }
        Command::Migrate => {
            databases.migrate().await?;
            println!("Database migrations completed.");
        }
        Command::Observe {
            command: ObserveCommand::Report { project },
        } => {
            let observe = Observe::new(databases.observe.clone());
            let environments = databases.access.project_environments(project).await?;
            print!(
                "{}",
                observe_report(&observe, project, &environments).await?
            );
        }
    }
    databases.close().await;
    Ok(())
}
