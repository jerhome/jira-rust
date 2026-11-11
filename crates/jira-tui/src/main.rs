//! jira-rust : TUI de gestion Scrum (backlog, sprint actif, versions) pour Jira Cloud.

mod app;
mod config;
mod doctor;
mod logging;

use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use color_eyre::eyre::Result;
use jira_client::mock::MockJira;
use jira_client::{JiraApi, JiraHttp};

#[derive(Parser, Debug)]
#[command(
    name = "jira-rust",
    version,
    about = "TUI Jira Cloud : backlog, sprint actif, versions"
)]
struct Cli {
    /// Utilise des données fictives en mémoire (aucun accès réseau).
    #[arg(long, global = true)]
    mock: bool,

    /// Fichier de configuration (défaut : ~/.config/jira-rust/config.toml).
    #[arg(long, global = true, value_name = "FICHIER")]
    config: Option<PathBuf>,

    /// Lecture seule : aucune écriture n'est envoyée à Jira (bloquée dans le client HTTP).
    #[arg(long, global = true)]
    read_only: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Vérifie la configuration et la connexion à Jira (appel de /myself).
    Doctor,
    /// Affiche le chemin du fichier de configuration et un exemple de contenu.
    ConfigPath,
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();
    let _log_guard = logging::init();

    match cli.command {
        Some(Command::ConfigPath) => {
            config::print_help(cli.config.as_deref());
            Ok(())
        }
        Some(Command::Doctor) => doctor::run(cli.mock, cli.config.as_deref()).await,
        None => {
            let api = build_api(cli.mock, cli.config.as_deref(), cli.read_only)?;
            app::run(api, cli.read_only).await
        }
    }
}

/// Construit l'implémentation de JiraApi : mock ou client HTTP depuis la config.
/// En lecture seule, le client HTTP refuse lui-même toute écriture ; avec `--mock`, seule la TUI
/// la bloque (données en mémoire, sans risque).
pub fn build_api(
    mock: bool,
    config_path: Option<&std::path::Path>,
    read_only: bool,
) -> Result<Arc<dyn JiraApi>> {
    if mock {
        return Ok(Arc::new(MockJira::new()));
    }
    let cfg = config::load(config_path)?;
    let client = JiraHttp::new(&cfg.base_url, cfg.email, cfg.token)?.with_read_only(read_only);
    Ok(Arc::new(client))
}
