use std::process::ExitCode;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

const ENV_INTERFACE: &str = "UC_INTEGRATION_INTERFACE";
const ENV_PORT: &str = "UC_INTEGRATION_HTTP_PORT";
const DEFAULT_INTERFACE: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 10791;

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let interface = std::env::var(ENV_INTERFACE).unwrap_or_else(|_| DEFAULT_INTERFACE.into());
    let port = std::env::var(ENV_PORT)
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(DEFAULT_PORT);
    let integration_addr = format!("{interface}:{port}");
    let link_addr = format!("{}:{}", r3cui_bridge::link::HOST, r3cui_bridge::link::PORT);

    let bound = tokio::try_join!(
        TcpListener::bind(&integration_addr),
        TcpListener::bind(&link_addr)
    );
    let (integration, link) = match bound {
        Ok(listeners) => listeners,
        Err(e) => {
            tracing::error!("cannot listen on {integration_addr} and {link_addr}: {e}");
            return ExitCode::FAILURE;
        }
    };
    tracing::info!("r3cui-bridge {} listening: Integration-API on {integration_addr}, bridge link on {link_addr}", r3cui_bridge::api::DRIVER_VERSION);
    r3cui_bridge::serve(integration, link).await;
    ExitCode::SUCCESS
}
