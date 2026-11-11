//! `jira-fake` : faux Jira Cloud en mémoire, sur 127.0.0.1 uniquement.

use std::time::Duration;

use clap::Parser;
use jira_fake::{Options, SUPPORTED_JQL, bind, serve};

/// Faux serveur Jira Cloud (REST v3 + Agile 1.0) en mémoire, pour tester jira-rust.
/// Écoute uniquement sur 127.0.0.1 ; les données sont perdues à l'arrêt.
#[derive(Parser)]
#[command(version)]
struct Cli {
    /// Port d'écoute (sur 127.0.0.1). 0 = port éphémère.
    #[arg(long, default_value_t = 8089)]
    port: u16,
    /// Latence ajoutée à chaque requête, en millisecondes.
    #[arg(long, default_value_t = 0)]
    latency_ms: u64,
    /// Fraction des requêtes (0.0 à 1.0) renvoyant 429 Too Many Requests.
    #[arg(long, default_value_t = 0.0, value_parser = parse_rate)]
    fail_rate: f64,
    /// Valeur de l'en-tête Retry-After (secondes) des réponses 429.
    #[arg(long, default_value_t = 1)]
    retry_after: u64,
    /// Refuse toutes les écritures (403), la recherche JQL restant autorisée.
    #[arg(long)]
    forbid_writes: bool,
    /// Ne journalise pas les requêtes sur stderr.
    #[arg(long, short)]
    quiet: bool,
}

fn parse_rate(s: &str) -> Result<f64, String> {
    let v: f64 = s.parse().map_err(|_| format!("nombre attendu : {s}"))?;
    if (0.0..=1.0).contains(&v) {
        Ok(v)
    } else {
        Err("la fraction doit être comprise entre 0.0 et 1.0".into())
    }
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let cli = Cli::parse();
    let listener = bind(cli.port).await?;
    let addr = listener.local_addr()?;
    eprintln!("jira-fake : faux Jira Cloud sur http://{addr} (Ctrl-C pour arrêter)");
    eprintln!("jira-fake : JQL acceptées : {SUPPORTED_JQL}");
    if cli.fail_rate > 0.0 {
        eprintln!(
            "jira-fake : 429 sur {:.0} % des requêtes (Retry-After: {})",
            cli.fail_rate * 100.0,
            cli.retry_after
        );
    }
    if cli.forbid_writes {
        eprintln!("jira-fake : écritures refusées (403)");
    }
    let opts = Options {
        latency: Duration::from_millis(cli.latency_ms),
        fail_rate: cli.fail_rate,
        retry_after_secs: cli.retry_after,
        forbid_writes: cli.forbid_writes,
        quiet: cli.quiet,
    };
    serve(listener, opts, async {
        let _ = tokio::signal::ctrl_c().await;
        eprintln!("jira-fake : arrêt");
    })
    .await
}
