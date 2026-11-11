//! `jira-rust doctor` : diagnostic de la configuration et de la connexion.

use std::path::Path;

use color_eyre::eyre::Result;

use crate::config::{self, TokenSource};

pub async fn run(mock: bool, config_path: Option<&Path>) -> Result<()> {
    println!("jira-rust {} — diagnostic", env!("CARGO_PKG_VERSION"));

    if !mock {
        let cfg = config::load(config_path)?;
        println!("  configuration : {}", cfg.path.display());
        println!("  instance      : {}", cfg.base_url);
        println!("  compte        : {}", cfg.email);
        println!(
            "  token         : présent ({})",
            match cfg.token_source {
                TokenSource::Env => config::TOKEN_ENV,
                TokenSource::File => "fichier",
            }
        );
    } else {
        println!("  mode          : mock (données fictives, aucun accès réseau)");
    }

    // doctor ne fait que lire : toujours en lecture seule.
    let api = crate::build_api(mock, config_path, true)?;
    print!("  connexion     : ");
    match api.myself().await {
        Ok(me) => {
            println!(
                "OK — connecté en tant que {} ({})",
                me.display_name, me.account_id
            );
            Ok(())
        }
        Err(e) => {
            println!("ÉCHEC");
            Err(e.into())
        }
    }
}
