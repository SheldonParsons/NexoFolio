use clap::{Parser, Subcommand};
use nexofolio_access_adapter::Postgres;
use nexofolio_backend::wiring::{Config, init_logging};

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
    let database = Postgres::new(
        &config.database_url,
        config.database_max_connections,
        config.database_timeout,
    )?;
    let _access = nexofolio_backend::wiring::build_access(&config, database.clone())?;
    match cli.command {
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
