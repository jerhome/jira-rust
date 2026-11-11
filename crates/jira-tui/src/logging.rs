//! Logs dans un fichier (jamais sur le terminal, qui appartient à la TUI).
//! Emplacement : ~/.local/share/jira-rust/jira-rust.log (dossier en 0700) ;
//! niveau via `JIRA_RUST_LOG` (défaut : info). Les crates HTTP/TLS restent plafonnées à `warn`.

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

const QUIET_DEPS: &str = "hyper=warn,hyper_util=warn,h2=warn,reqwest=warn,rustls=warn";

pub fn init() -> Option<WorkerGuard> {
    let dir = directories::ProjectDirs::from("", "", "jira-rust")?
        .data_local_dir()
        .to_path_buf();
    std::fs::create_dir_all(&dir).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).ok()?;
    }
    let appender = tracing_appender::rolling::never(dir, "jira-rust.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let user = std::env::var("JIRA_RUST_LOG").unwrap_or_else(|_| "info".into());
    let filter = EnvFilter::try_new(format!("{user},{QUIET_DEPS}"))
        .unwrap_or_else(|_| EnvFilter::new(format!("info,{QUIET_DEPS}")));
    tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .with_env_filter(filter)
        .try_init()
        .ok()?;
    Some(guard)
}
