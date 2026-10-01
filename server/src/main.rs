use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "pulse-app-server")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the HTTP + gateway server
    Serve,
    /// Print a fresh single-use invite code (bootstraps the first account)
    CreateInvite,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,sqlx=warn".into()),
        )
        .init();
    let cfg = pulse_server::config::Config::from_env()?;
    match Cli::parse().cmd {
        Cmd::Serve => pulse_server::serve(cfg).await,
        Cmd::CreateInvite => {
            let db = pulse_server::db::connect(&cfg.db_url).await?;
            println!("{}", pulse_server::auth::invites::create(&db, None).await?);
            Ok(())
        }
    }
}
