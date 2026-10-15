use clap::{Parser, Subcommand};
use nexofolio_backend::wiring::{Config, Databases, build_api, init_logging, observe_report};
use nexofolio_common::{ProjectId, TokenId, UserId};
use nexofolio_observe::Observe;

#[derive(Parser)]
#[command(about = "NexoFolio foundation administration")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// One bounded project batch. Preview by default; first execution starts a 7-day grace.
    RetainHistory {
        #[arg(long)]
        project: ProjectId,
        #[arg(long)]
        execute: bool,
        #[arg(long, default_value = "10")]
        limit: usize,
        #[arg(long)]
        after: Option<uuid::Uuid>,
    },
    CheckConfig,
    /// Write a new MCP credential to a protected local file; stdout contains only its ID.
    IssueMcpToken {
        #[arg(long)]
        user: UserId,
        #[arg(long)]
        output: std::path::PathBuf,
        #[arg(long, default_value = "7")]
        days: u32,
    },
    RevokeMcpToken {
        #[arg(long)]
        id: TokenId,
    },
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
    if let Command::RetainHistory {
        project,
        execute,
        limit,
        after,
    } = cli.command
    {
        // Dedicated URL; never load ordinary application/model configuration here.
        if execute && std::env::var("NEXOFOLIO_RETENTION_ENABLED").as_deref() != Ok("1") {
            return Err("execution requires NEXOFOLIO_RETENTION_ENABLED=1".into());
        }
        let url = nexofolio_common::Secret::new(std::env::var("NEXOFOLIO_RETENTION_DATABASE_URL")?);
        let store = nexofolio_knowledge_adapter::PostgresKnowledge::new(
            &url,
            1,
            std::time::Duration::from_secs(2),
        )?;
        let started = std::time::Instant::now();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            store.retain_history(
                project,
                chrono::DateTime::from(std::time::SystemTime::now()),
                after,
                limit,
                execute,
            ),
        )
        .await;
        store.close().await;
        let report = result??;
        println!(
            "{}",
            serde_json::json!({"elapsed_ms":started.elapsed().as_millis(),"report":report})
        );
        return Ok(());
    }
    let config = Config::from_env()?;
    init_logging(&config.log_filter)?;
    let databases = Databases::new(&config)?;
    let _api = build_api(&config, &databases)?;
    match cli.command {
        Command::RetainHistory { .. } => unreachable!(),
        Command::IssueMcpToken { user, output, days } => {
            use std::io::Write;
            // Create the file first: an existing file never gets overwritten.
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&output)?;
            match databases.access.issue_mcp_token(user, days).await {
                Ok((id, token)) => {
                    if let Err(error) = file.write_all(token.expose().as_bytes()) {
                        let _ = databases.access.revoke_mcp_token(id).await;
                        let _ = std::fs::remove_file(&output);
                        return Err(error.into());
                    }
                    println!("MCP token {id} saved to protected file.");
                }
                Err(error) => {
                    let _ = std::fs::remove_file(&output);
                    return Err(error.into());
                }
            }
        }
        Command::RevokeMcpToken { id } => {
            databases.access.revoke_mcp_token(id).await?;
            println!("MCP token revoked.");
        }
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
