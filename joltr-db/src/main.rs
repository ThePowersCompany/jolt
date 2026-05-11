//! `joltr-db` CLI binary (JOLTR-RS-104).
//!
//! Currently exposes one subcommand: `joltr-db migrate new <name>` —
//! scaffolds a new empty migration file at
//! `migrations/<YYYYMMDDHHMMSS>_<name>.sql`. The actual file creation
//! lives in the library as [`joltr_db::create_migration_file`] (see lib
//! module docs decisions 49–53); this binary is a thin clap layer that
//! parses argv and supplies `chrono::Utc::now()`.

use std::process::ExitCode;

use clap::{Parser, Subcommand};

/// `joltr-db` — Postgres migration tooling for the JoltR framework.
#[derive(Parser)]
#[command(name = "joltr-db", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Manage migration files.
    Migrate {
        #[command(subcommand)]
        action: MigrateAction,
    },
}

#[derive(Subcommand)]
enum MigrateAction {
    /// Create a new empty migration file in `./migrations`.
    New {
        /// Short identifier for the migration (e.g. `add_users`).
        name: String,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Migrate { action } => match action {
            MigrateAction::New { name } => {
                // Decision 50: UTC, threaded as a parameter into the
                // library function so tests can pin a deterministic
                // timestamp. Real runs supply `chrono::Utc::now()`
                // here at the call site.
                let now = chrono::Utc::now();
                match joltr_db::create_migration_file("migrations", &name, now) {
                    Ok(path) => {
                        println!("created {}", path.display());
                        ExitCode::SUCCESS
                    }
                    Err(err) => {
                        eprintln!("error: {err}");
                        ExitCode::FAILURE
                    }
                }
            }
        },
    }
}
