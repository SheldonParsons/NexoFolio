//! Isolated downloads-only server for frontend verification; no database or credentials.
use nexofolio_backend::http::downloads;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let port = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "18981".into())
        .parse::<u16>()?;
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?;
    println!(
        "Downloads preview: http://{}/v1/downloads",
        listener.local_addr()?
    );
    axum::serve(listener, downloads::routes())
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
