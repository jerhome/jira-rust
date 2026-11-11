//! Interface TUI (phase 2). Point d'entrée imposé : `run(api)`.
//!
//! Architecture :
//! - `state` : état pur (`App`), indépendant du terminal ;
//! - `update` : logique `update(app, event) -> Vec<Effect>`, sans E/S ;
//! - `runtime` : exécution des effets contre `JiraApi` (seul module qui appelle l'API) ;
//! - `ui` : rendu pur `&App -> Frame` ;
//! - ce module : boucle d'événements (thread clavier + canal `tokio::mpsc` + tâches tokio).

mod action;
mod dates;
mod form;
mod runtime;
mod state;
mod text;
mod ui;
mod update;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod e2e_tests;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use color_eyre::eyre::{Result, eyre};
use crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind};
use jiff::tz::TimeZone;
use jira_client::JiraApi;
use ratatui::DefaultTerminal;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use action::{AppEvent, Effect, Msg};
use state::App;

/// Intervalle d'attente du clavier ; un `Tick` est émis à chaque attente vide (animation du spinner).
const POLL: Duration = Duration::from_millis(200);

/// Lance la TUI. Contrat : ne fait aucun appel réseau autrement que via `api`.
pub async fn run(api: Arc<dyn JiraApi>, read_only: bool) -> Result<()> {
    let mut app = App::new(dates::today(), TimeZone::system());
    app.read_only = read_only;
    let (tx, mut rx) = unbounded_channel();
    let stop = Arc::new(AtomicBool::new(false));

    let mut terminal = ratatui::init();
    enable_bracketed_paste();
    let input = spawn_input_thread(tx.clone(), stop.clone());
    let result = event_loop(&mut terminal, &mut app, &api, &tx, &mut rx).await;
    stop.store(true, Ordering::Relaxed);
    disable_bracketed_paste();
    ratatui::restore();
    // Le thread clavier se termine au plus tard après un intervalle de poll.
    let _ = input.join();

    result?;
    match app.fatal {
        Some(e) => Err(eyre!("arrêt sur erreur interne : {e}")),
        None => Ok(()),
    }
}

/// Collage « entre crochets » : le terminal livre un texte collé en un seul `Event::Paste` au
/// lieu d'une rafale de touches (un saut de ligne collé ne doit jamais valider un formulaire).
/// Désactivé à la restauration, y compris sur panique (hook chaîné devant celui de ratatui).
fn enable_bracketed_paste() {
    if let Err(e) = crossterm::execute!(std::io::stdout(), EnableBracketedPaste) {
        tracing::warn!(error = %e, "bracketed paste indisponible");
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        disable_bracketed_paste();
        previous(info);
    }));
}

fn disable_bracketed_paste() {
    let _ = crossterm::execute!(std::io::stdout(), DisableBracketedPaste);
}

async fn event_loop(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    api: &Arc<dyn JiraApi>,
    tx: &UnboundedSender<AppEvent>,
    rx: &mut UnboundedReceiver<AppEvent>,
) -> Result<()> {
    for effect in update::init(app) {
        dispatch(api, tx, effect);
    }
    while !app.should_quit {
        terminal.draw(|f| ui::draw(f, app))?;
        // Une confirmation n'est validable qu'une fois réellement affichée (et après un délai,
        // voir `state::CONFIRM_DELAY`) : les frappes déjà en file ne peuvent pas la valider.
        app.mark_drawn(Instant::now());
        let Some(event) = rx.recv().await else { break };
        let mut effects = update::update(app, event);
        // Traiter d'un coup les événements déjà arrivés avant de redessiner.
        while !app.should_quit
            && let Ok(event) = rx.try_recv()
        {
            effects.extend(update::update(app, event));
        }
        for effect in effects {
            dispatch(api, tx, effect);
        }
    }
    Ok(())
}

/// Exécute un effet dans une tâche tokio ; le résultat revient par le canal.
/// Une panique de la tâche est convertie en [`Msg::TaskFailed`].
pub(crate) fn dispatch(api: &Arc<dyn JiraApi>, tx: &UnboundedSender<AppEvent>, effect: Effect) {
    let api = Arc::clone(api);
    let task = tokio::spawn(async move { runtime::execute(api.as_ref(), effect).await });
    let tx = tx.clone();
    tokio::spawn(async move {
        let msg = match task.await {
            Ok(msg) => msg,
            Err(e) => Msg::TaskFailed(format!("tâche asynchrone interrompue : {e}")),
        };
        // Récepteur fermé = application en cours d'arrêt : rien à faire.
        let _ = tx.send(AppEvent::Msg(msg));
    });
}

/// Lit le clavier dans un thread dédié (crossterm est bloquant) et alimente le canal.
fn spawn_input_thread(tx: UnboundedSender<AppEvent>, stop: Arc<AtomicBool>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            let event = match event::poll(POLL) {
                Ok(false) => AppEvent::Tick(dates::today()),
                Ok(true) => match event::read() {
                    Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => {
                        AppEvent::Key(k, Instant::now())
                    }
                    Ok(Event::Paste(text)) => AppEvent::Paste(text, Instant::now()),
                    Ok(Event::Resize(..)) => AppEvent::Resize,
                    Ok(_) => continue,
                    Err(e) => AppEvent::InputError(e.to_string()),
                },
                Err(e) => AppEvent::InputError(e.to_string()),
            };
            let fatal = matches!(event, AppEvent::InputError(_));
            if tx.send(event).is_err() || fatal {
                break;
            }
        }
    })
}
