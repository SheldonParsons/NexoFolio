use clap::{Parser, Subcommand};
use nexofolio_backend::wiring::{Config, Databases, build_api, init_logging};

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
    }
    databases.close().await;
    Ok(())
}
