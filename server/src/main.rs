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
    /// Apply any pending database migrations and exit (needs only PULSE_DB_URL)
    Migrate,
    /// Check the clips storage settings: PUT, HEAD, GET and DELETE a tiny probe object (needs
    /// only the PULSE_CLIPS_S3_* variables)
    ClipsCheck,
    /// Print a fresh single-use invite code (bootstraps the first account)
    CreateInvite,
    /// Write a consistent snapshot of the database to PATH (must not exist yet)
    Backup { path: std::path::PathBuf },
    /// Turn off two-factor authentication for a user who is locked out (needs no key)
    #[command(name = "reset-2fa")]
    Reset2fa { username: String },
    /// Set a user's password, read as one line from stdin (never an argument), and sign out all
    /// of their sessions
    ResetPassword { username: String },
    /// Sign a user out everywhere: delete all of their sessions
    RevokeSessions { username: String },
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
    let cmd = Cli::parse().cmd;
    // `migrate` reads only the database URL: no LiveKit settings are needed to run it.
    if let Cmd::Migrate = cmd {
        let url = std::env::var("PULSE_DB_URL").unwrap_or_else(|_| "sqlite://pulse.db".into());
        let version = pulse_server::db::migrate(&url).await?;
        println!("migrated: {version}");
        return Ok(());
    }
    // Likewise `clips-check` needs only the storage settings.
    if let Cmd::ClipsCheck = cmd {
        let Some(store) = pulse_server::config::ClipStoreConfig::from_env()? else {
            anyhow::bail!("clips storage isn't configured: set the PULSE_CLIPS_S3_* variables");
        };
        let r2 = pulse_server::media::r2::R2::new(&store)?;
        return match r2.probe().await {
            Ok(()) => {
                println!("clips storage ok");
                Ok(())
            }
            Err(step) => anyhow::bail!("{step}"),
        };
    }
    let cfg = pulse_server::config::Config::from_env()?;
    match cmd {
        Cmd::Migrate | Cmd::ClipsCheck => unreachable!("handled above"),
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
        Cmd::Reset2fa { username } => {
            let db = pulse_server::db::connect(&cfg.db_url).await?;
            let name = pulse_server::auth::two_factor::reset(&db, &username).await?;
            println!(
                "two-factor turned off for {name}: they can sign in with their password and set it up again"
            );
            Ok(())
        }
        // The running server isn't told: REST calls with a deleted session fail at once, but its
        // open real-time connections and calls stay until they drop (restarting it ends them).
        Cmd::ResetPassword { username } => {
            let password = pulse_server::auth::routes::read_password_line(std::io::stdin().lock())?;
            let db = pulse_server::db::connect(&cfg.db_url).await?;
            let (name, n) =
                pulse_server::auth::routes::reset_password(&db, &username, &password).await?;
            println!("password reset for {name}; signed out {n} {}", sessions(n));
            Ok(())
        }
        Cmd::RevokeSessions { username } => {
            let db = pulse_server::db::connect(&cfg.db_url).await?;
            let (name, n) = pulse_server::auth::routes::revoke_sessions(&db, &username).await?;
            println!("signed out {n} {} of {name}", sessions(n));
            Ok(())
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

fn sessions(n: u64) -> &'static str {
    if n == 1 { "session" } else { "sessions" }
}
