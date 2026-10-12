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
    /// Write a consistent snapshot of the database to PATH (must not exist yet)
    Backup { path: std::path::PathBuf },
    /// List the newest bug/crash reports
    Reports {
        #[arg(long, default_value_t = 20)]
        limit: i64,
    },
    /// Show one report (by the short id from `reports`) and mark it seen
    Report {
        id: String,
        /// Also print the attached app log
        #[arg(long)]
        log: bool,
    },
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
        Cmd::Backup { path } => {
            let db = pulse_server::db::connect(&cfg.db_url).await?;
            pulse_server::db::backup(&db, &path).await
        }
        Cmd::Reports { limit } => {
            let db = pulse_server::db::connect(&cfg.db_url).await?;
            let rows = pulse_server::reports::store::list(&db, limit).await?;
            if rows.is_empty() {
                println!("no reports");
            }
            for r in rows {
                println!(
                    "{}  {}  {:<16} {:<15} {:3}  {}",
                    r.short,
                    &r.created_at[..16],
                    r.username,
                    r.kind,
                    if r.seen { "" } else { "new" },
                    r.headline
                );
            }
            Ok(())
        }
        Cmd::Report { id, log } => {
            let db = pulse_server::db::connect(&cfg.db_url).await?;
            let r = pulse_server::reports::store::find(&db, &id).await?;
            let l = &r.listed;
            println!(
                "#{}  {}  {}  {}\n",
                l.short, l.created_at, l.username, l.kind
            );
            println!(
                "{}\n",
                if r.text.is_empty() {
                    "(no text)"
                } else {
                    &r.text
                }
            );
            println!("{}", serde_json::to_string_pretty(&r.meta)?);
            if log {
                println!("\n----- app log -----\n{}", r.log);
            } else {
                println!("\n({} KB of log: add --log)", r.log.len() / 1024);
            }
            pulse_server::reports::store::mark_seen(&db, &l.id).await?;
            Ok(())
        }
    }
}
