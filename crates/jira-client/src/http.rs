//! Client HTTP Jira Cloud (REST API plateforme v3 + Jira Software Agile 1.0).
//!
//! Choix principaux (voir aussi `docs/api/*.md`) :
//! - **Lectures d'issues** : uniquement `POST /rest/api/3/search/jql` (pagination `nextPageToken`),
//!   avec une liste `fields` explicite. Un seul format d'issue (v3, description ADF) ; aucun endpoint
//!   agile de lecture d'issues (dépréciés).
//!   - sprint : `sprint = {id} ORDER BY Rank ASC` ;
//!   - backlog : filtre du board (`GET /rest/agile/1.0/board/{id}/configuration` -> `filter.id`) puis
//!     `filter = {filterId} AND (sprint is EMPTY OR (sprint not in openSprints() AND sprint not in
//!     futureSprints())) AND statusCategory != Done AND issuetype not in subtaskIssueTypes()
//!     ORDER BY Rank ASC`. Sur Jira Cloud, `openSprints()` peut ne couvrir que les sprints actifs :
//!     l'exclusion explicite de `futureSprints()` évite qu'une issue d'un sprint futur apparaisse
//!     aussi dans le backlog (redondante, donc sans effet, si `openSprints()` couvre les futurs).
//!     Limites : c'est une approximation du backlog Jira. Jira masque les issues dont le statut est mappé
//!     sur la dernière colonne du board (et non la catégorie Done) ; les épopées (affichées à part dans
//!     l'UI Jira) sont incluses ; les sous-tâches sont exclues (elles suivent leur parent) ; le
//!     « sous-filtre » des boards Kanban n'existe pas pour Scrum.
//!   - version : `project = "{key}" AND fixVersion = {versionId} ORDER BY Rank ASC`.
//! - **Champs custom** (story points, sprint) découverts une fois via `GET /rest/api/3/field` et mis en
//!   cache. Story points : `estimation.field.fieldId` de la configuration **du board concerné**
//!   (cache par board), sinon schéma `jsw-story-points`, sinon float nommé « Story Points ».
//!   En lecture, tous les champs candidats sont demandés et le premier non nul (dans cet ordre) gagne.
//!   Board concerné : backlog = board demandé ; sprint = board dont la liste de sprints l'a fait
//!   connaître ; version = dernier board lu du projet. En écriture : `update_issue` = board du dernier
//!   contexte de lecture de l'issue, sinon du projet ; `create_issue` = dernier board lu du projet ;
//!   à défaut, catalogue global (`jsw-story-points` puis « Story Points »).
//! - **Cohérence éventuelle** : les ids des issues touchées par les dernières écritures (50 au plus,
//!   60 s) sont passés en `reconcileIssues` aux `search/jql` suivantes.
//! - **Écritures** par lots de 50 (sprint, backlog, rang), non atomiques au-delà.
//! - **429** (et **503 avec `Retry-After`**) : nouvel essai (au plus 4 tentatives) uniquement pour les
//!   lectures (GET et `search/jql`), délai = max(`Retry-After`, backoff exponentiel avec jitter) ; si `Retry-After` dépasse le plafond,
//!   l'erreur est renvoyée immédiatement. Jamais de nouvel essai sur une écriture.
//! - **Sécurité** : pas de redirection suivie (3xx = erreur), aucune URL hors de l'origine de base,
//!   segments dynamiques encodés, `http://` refusé hors loopback.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use reqwest::{Client, Method, StatusCode, header, redirect};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};
use tokio::sync::OnceCell;
use url::Url;

use crate::model::*;
use crate::{Error, JiraApi, Result, SecretString, adf, guard, timeutil};

/// Taille de page des listes paginées par offset.
const PAGE_SIZE: u32 = 50;
/// Taille de page demandée à `search/jql` (Jira peut renvoyer moins).
const SEARCH_PAGE_SIZE: u32 = 100;
/// Taille maximale d'un lot d'écriture agile (sprint, backlog, rang).
const WRITE_BATCH: usize = 50;
/// Garde-fou contre une pagination qui ne se termine pas.
const MAX_PAGES: usize = 1000;
/// `reconcileIssues` : 50 ids au plus (limite Jira).
const RECONCILE_MAX: usize = 50;
/// Durée pendant laquelle une issue écrite est passée en `reconcileIssues`.
const RECONCILE_WINDOW: Duration = Duration::from_secs(60);
/// Borne des caches clé d'issue -> id / board (vidés au-delà).
const CONTEXT_MAX_ISSUES: usize = 10_000;

const SPRINT_SCHEMA: &str = "com.pyxis.greenhopper.jira:gh-sprint";
const JSW_POINTS_SCHEMA: &str = "com.pyxis.greenhopper.jira:jsw-story-points";
const FLOAT_SCHEMA: &str = "com.atlassian.jira.plugin.system.customfieldtypes:float";

/// Politique de nouvel essai sur 429 (lectures uniquement).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Nombre total de tentatives (1 = aucun nouvel essai).
    pub max_attempts: u32,
    /// Délai de base du backoff exponentiel (doublé à chaque tentative, jitter 0,7–1,3).
    pub base_delay: Duration,
    /// Plafond d'attente : un `Retry-After` supérieur fait échouer immédiatement.
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 4,
            base_delay: Duration::from_secs(2),
            max_delay: Duration::from_secs(30),
        }
    }
}

impl RetryPolicy {
    /// Aucun nouvel essai : le 429 est renvoyé tel quel.
    pub fn none() -> Self {
        Self {
            max_attempts: 1,
            ..Self::default()
        }
    }

    fn backoff(&self, attempt: u32) -> Duration {
        let exp = self
            .base_delay
            .saturating_mul(1u32 << attempt.saturating_sub(1).min(16));
        let capped = exp.min(self.max_delay);
        capped.mul_f64(jitter_factor()).min(self.max_delay)
    }
}

/// Facteur pseudo-aléatoire dans [0,7 ; 1,3] (pas besoin d'un PRNG cryptographique).
fn jitter_factor() -> f64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default(),
    );
    0.7 + (h.finish() % 10_000) as f64 / 10_000.0 * 0.6
}

/// Champs custom découverts via `/rest/api/3/field`.
#[derive(Debug, Clone, Default)]
struct FieldCatalog {
    sprint: Option<String>,
    points_jsw: Option<String>,
    points_legacy: Option<String>,
}

/// Contexte mémorisé au fil des lectures/écritures (partagé entre clones du client).
#[derive(Debug, Default)]
struct Context {
    /// board -> champ d'estimation (`None` : configuration lue, estimation hors champ).
    board_points: HashMap<u64, Option<String>>,
    /// board -> clé de projet (liste des boards).
    board_project: HashMap<u64, String>,
    /// board -> type (`scrum`, `simple`).
    board_type: HashMap<u64, String>,
    /// sprint -> board dont la liste de sprints l'a renvoyé.
    sprint_board: HashMap<u64, u64>,
    /// clé de projet (majuscules) -> dernier board lu (backlog / sprints / issues de sprint).
    project_board: HashMap<String, u64>,
    /// clé d'issue (majuscules) -> board du dernier contexte de lecture.
    issue_board: HashMap<String, u64>,
    /// clé d'issue (majuscules) -> id numérique (pour `reconcileIssues`).
    issue_ids: HashMap<String, u64>,
    /// Ids des issues récemment écrites (plus ancienne en tête).
    recent_writes: VecDeque<(u64, Instant)>,
}

/// Clé de projet d'une clé d'issue (`DEMO-12` -> `DEMO`).
fn project_of(issue_key: &str) -> Option<String> {
    issue_key
        .trim()
        .rsplit_once('-')
        .map(|(p, _)| p.to_ascii_uppercase())
        .filter(|p| !p.is_empty())
}

impl Context {
    fn note_written(&mut self, id: u64, window: Duration) {
        let now = Instant::now();
        self.recent_writes
            .retain(|(i, at)| *i != id && now.duration_since(*at) < window);
        self.recent_writes.push_back((id, now));
        while self.recent_writes.len() > RECONCILE_MAX {
            self.recent_writes.pop_front();
        }
    }

    fn reconcile_ids(&mut self, window: Duration) -> Vec<u64> {
        let now = Instant::now();
        self.recent_writes
            .retain(|(_, at)| now.duration_since(*at) < window);
        self.recent_writes.iter().map(|(id, _)| *id).collect()
    }

    fn note_issue(&mut self, key: &str, id: Option<u64>, board: Option<u64>) {
        if self.issue_ids.len() >= CONTEXT_MAX_ISSUES {
            self.issue_ids.clear();
        }
        if self.issue_board.len() >= CONTEXT_MAX_ISSUES {
            self.issue_board.clear();
        }
        let key = key.trim().to_ascii_uppercase();
        if let Some(id) = id {
            self.issue_ids.insert(key.clone(), id);
        }
        if let Some(board) = board {
            if let Some(project) = project_of(&key) {
                self.project_board.insert(project, board);
            }
            self.issue_board.insert(key, board);
        }
    }

    /// Board dont le champ d'estimation s'applique à l'issue : contexte de lecture de l'issue, sinon
    /// dernier board lu de son projet.
    fn board_for_issue(&self, key: &str) -> Option<u64> {
        let key = key.trim().to_ascii_uppercase();
        self.issue_board
            .get(&key)
            .copied()
            .or_else(|| project_of(&key).and_then(|p| self.project_board.get(&p).copied()))
    }

    fn board_for_project(&self, project_key: &str) -> Option<u64> {
        self.project_board
            .get(&project_key.trim().to_ascii_uppercase())
            .copied()
    }

    /// Le board devient le contexte de lecture de son projet (si connu).
    fn touch_board(&mut self, board: u64) {
        if let Some(project) = self.board_project.get(&board).cloned() {
            self.project_board.insert(project, board);
        }
    }
}

#[derive(Debug, Clone)]
pub struct JiraHttp {
    client: Client,
    base: Url,
    email: String,
    token: SecretString,
    retry: RetryPolicy,
    catalog: Arc<OnceCell<FieldCatalog>>,
    /// Contexte (champs d'estimation par board, ids d'issues, écritures récentes).
    context: Arc<Mutex<Context>>,
    /// Fenêtre de `reconcileIssues` après une écriture.
    reconcile_window: Duration,
    /// Lecture seule : toute requête autre que GET ou `POST search/jql` est refusée avant envoi.
    read_only: bool,
}

/// Chemin de la recherche JQL : seul POST autorisé en lecture seule (c'est une lecture).
const SEARCH_JQL_PATH: &str = "/rest/api/3/search/jql";

impl JiraHttp {
    /// `base_url` : racine de l'instance, ex. `https://<site>.atlassian.net`.
    /// Refuse : schéma autre que http(s), `http://` hors loopback, identifiants dans l'URL ;
    /// échoue aussi si le mode hors-ligne est actif et que l'hôte n'est pas local.
    /// Les messages d'erreur ne recopient jamais l'URL brute (elle peut contenir un secret).
    pub fn new(base_url: &str, email: impl Into<String>, token: SecretString) -> Result<Self> {
        // Identifiants d'abord, avant toute autre analyse ou message.
        if raw_url_has_userinfo(base_url) {
            return Err(Error::InvalidUrl(
                "identifiants interdits dans l'URL (utiliser e-mail + token)".into(),
            ));
        }
        let mut base = Url::parse(base_url.trim())
            .map_err(|e| Error::InvalidUrl(format!("URL de base illisible ({e})")))?;
        if !base.username().is_empty() || base.password().is_some() {
            return Err(Error::InvalidUrl(
                "identifiants interdits dans l'URL (utiliser e-mail + token)".into(),
            ));
        }
        match base.scheme() {
            "https" => {}
            "http" if guard::is_loopback(&base) => {}
            "http" => {
                return Err(Error::InvalidUrl(format!(
                    "{} (http non chiffré refusé hors localhost : utiliser https)",
                    url_origin(&base)
                )));
            }
            _ => {
                return Err(Error::InvalidUrl(format!(
                    "schéma « {} » non supporté (utiliser https)",
                    base.scheme()
                )));
            }
        }
        if base.host().is_none() {
            return Err(Error::InvalidUrl("hôte absent de l'URL de base".into()));
        }
        base.set_query(None);
        base.set_fragment(None);
        if !base.path().ends_with('/') {
            let path = format!("{}/", base.path());
            base.set_path(&path);
        }
        guard::check(&base)?;

        let client = Client::builder()
            .user_agent(concat!("jira-rust/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(30))
            // Jamais de redirection : un 302 contournerait le garde-fou et l'origine fixée.
            .redirect(redirect::Policy::none())
            .build()
            .map_err(|e| Error::Transport(e.to_string()))?;

        Ok(Self {
            client,
            base,
            email: email.into(),
            token,
            retry: RetryPolicy::default(),
            catalog: Arc::new(OnceCell::new()),
            context: Arc::new(Mutex::new(Context::default())),
            reconcile_window: RECONCILE_WINDOW,
            read_only: false,
        })
    }

    /// Fenêtre pendant laquelle une issue écrite est passée en `reconcileIssues` aux recherches
    /// suivantes (60 s par défaut ; `Duration::ZERO` désactive).
    pub fn with_reconcile_window(mut self, window: Duration) -> Self {
        self.reconcile_window = window;
        self
    }

    /// Active la lecture seule : les écritures échouent avec [`Error::ReadOnly`] sans aucune
    /// requête réseau. Seuls GET et `POST /rest/api/3/search/jql` sont envoyés.
    pub fn with_read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Contrôle lecture seule, fail-closed : tout ce qui n'est pas explicitement une lecture est refusé.
    fn check_read_only(&self, method: &Method, url: &Url) -> Result<()> {
        if !self.read_only || *method == Method::GET {
            return Ok(());
        }
        let search = *method == Method::POST
            && url.query().is_none()
            && url
                .path()
                .strip_prefix(self.base.path().trim_end_matches('/'))
                .is_some_and(|p| p == SEARCH_JQL_PATH);
        if search {
            return Ok(());
        }
        tracing::warn!(%method, path = url.path(), "écriture bloquée (lecture seule)");
        Err(Error::ReadOnly(format!("{method} {}", url.path())))
    }

    /// Remplace la politique de nouvel essai sur 429.
    pub fn with_retry_policy(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    pub fn base_url(&self) -> &Url {
        &self.base
    }

    /// Construit l'URL d'un endpoint à partir de segments, chacun encodé (`/`, `?`, `#`, `%` compris).
    fn endpoint(&self, segments: &[&str]) -> Result<Url> {
        let mut url = self.base.clone();
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|_| Error::InvalidUrl(self.base.to_string()))?;
            path.pop_if_empty();
            for seg in segments {
                if seg.is_empty() || *seg == "." || *seg == ".." {
                    return Err(Error::Invalid(format!(
                        "segment d'URL invalide : « {seg} »"
                    )));
                }
                path.push(seg);
            }
        }
        self.ensure_same_origin(&url)?;
        Ok(url)
    }

    fn ensure_same_origin(&self, url: &Url) -> Result<()> {
        if url.scheme() != self.base.scheme()
            || url.host() != self.base.host()
            || url.port_or_known_default() != self.base.port_or_known_default()
            || !url.path().starts_with(self.base.path())
        {
            return Err(Error::InvalidUrl(format!(
                "URL hors de l'instance configurée : {}",
                url.path()
            )));
        }
        Ok(())
    }

    /// Exécute une requête ; renvoie le statut (2xx) et le corps brut.
    /// `retryable` : nouvel essai sur 429 (à réserver aux lectures).
    async fn send(
        &self,
        method: Method,
        url: Url,
        body: Option<&Value>,
        retryable: bool,
    ) -> Result<(StatusCode, String)> {
        self.ensure_same_origin(&url)?;
        self.check_read_only(&method, &url)?;
        let max_attempts = if retryable {
            self.retry.max_attempts.max(1)
        } else {
            1
        };
        let mut attempt = 0;
        loop {
            attempt += 1;
            tracing::debug!(%method, path = url.path(), attempt, "requête Jira");
            let mut req = self
                .client
                .request(method.clone(), url.clone())
                .basic_auth(&self.email, Some(self.token.expose()))
                .header(header::ACCEPT, "application/json");
            if let Some(body) = body {
                req = req.json(body);
            }
            let resp = req
                .send()
                .await
                .map_err(|e| Error::Transport(e.without_url().to_string()))?;
            let status = resp.status();
            let retry_after = resp
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(parse_retry_after);
            let location = resp
                .headers()
                .get(header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            let content_type = resp
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(str::to_ascii_lowercase);
            let text = resp
                .text()
                .await
                .map_err(|e| Error::Transport(e.without_url().to_string()))?;

            if status.is_success() {
                return Ok((status, text));
            }
            // 429, ou 503 transitoire accompagné de `Retry-After` (lectures uniquement).
            let retry_status = status == StatusCode::TOO_MANY_REQUESTS
                || (status == StatusCode::SERVICE_UNAVAILABLE && retry_after.is_some());
            if retry_status && attempt < max_attempts {
                let too_long = retry_after.is_some_and(|d| d > self.retry.max_delay);
                if !too_long {
                    let delay = retry_after
                        .unwrap_or_default()
                        .max(self.retry.backoff(attempt));
                    tracing::warn!(
                        status = status.as_u16(),
                        path = url.path(),
                        attempt,
                        delay_ms = delay.as_millis() as u64,
                        "Jira surchargé : nouvel essai"
                    );
                    tokio::time::sleep(delay).await;
                    continue;
                }
            }
            tracing::warn!(
                status = status.as_u16(),
                path = url.path(),
                "réponse Jira en erreur"
            );
            return Err(error_from_response(
                status,
                retry_after,
                content_type.as_deref(),
                location.as_deref(),
                &text,
            ));
        }
    }

    async fn get_json<T: DeserializeOwned>(&self, url: Url) -> Result<T> {
        let ctx = url.path().to_owned();
        let (_, text) = self.send(Method::GET, url, None, true).await?;
        decode(&text, &ctx)
    }

    /// Écriture renvoyant un objet JSON (jamais retentée).
    async fn write_json<T: DeserializeOwned>(
        &self,
        method: Method,
        url: Url,
        body: &Value,
    ) -> Result<T> {
        let ctx = url.path().to_owned();
        let (_, text) = self.send(method, url, Some(body), false).await?;
        decode(&text, &ctx)
    }

    /// Écriture sans contenu utile en retour (204 ou corps ignoré ; jamais retentée).
    async fn write_no_content(&self, method: Method, url: Url, body: &Value) -> Result<StatusCode> {
        let (status, _) = self.send(method, url, Some(body), false).await?;
        Ok(status)
    }

    /// Pagination offset (`startAt`/`maxResults`) jusqu'à `isLast` (ou `total`, ou page vide).
    async fn get_all<T: DeserializeOwned>(&self, url: Url) -> Result<Vec<T>> {
        let mut out = Vec::new();
        let mut start_at: u64 = 0;
        for _ in 0..MAX_PAGES {
            let mut page_url = url.clone();
            page_url
                .query_pairs_mut()
                .append_pair("startAt", &start_at.to_string())
                .append_pair("maxResults", &PAGE_SIZE.to_string());
            let page: OffsetPage<T> = self.get_json(page_url).await?;
            let n = page.values.len() as u64;
            out.extend(page.values);
            let done = match page.is_last {
                Some(true) => true,
                _ if n == 0 => true,
                Some(false) => false,
                None => page
                    .total
                    .is_none_or(|total| page.start_at.unwrap_or(start_at) + n >= total),
            };
            if done {
                return Ok(out);
            }
            start_at = page.start_at.unwrap_or(start_at) + n;
        }
        Err(Error::Decode(format!(
            "{} : pagination interrompue après {MAX_PAGES} pages",
            url.path()
        )))
    }

    // --- Champs custom -----------------------------------------------------------------------

    async fn catalog(&self) -> Result<&FieldCatalog> {
        self.catalog
            .get_or_try_init(|| async {
                let url = self.endpoint(&["rest", "api", "3", "field"])?;
                let fields: Vec<FieldDto> = self.get_json(url).await?;
                Ok(FieldCatalog::from_fields(&fields))
            })
            .await
    }

    /// Contexte partagé (verrou court, jamais tenu à travers un `await`).
    fn ctx(&self) -> MutexGuard<'_, Context> {
        self.context
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    async fn board_config(&self, board_id: u64) -> Result<BoardConfigDto> {
        let id = board_id.to_string();
        let url = self.endpoint(&["rest", "agile", "1.0", "board", &id, "configuration"])?;
        let cfg: BoardConfigDto = self.get_json(url).await?;
        let field = cfg.estimation_field().map(str::to_owned);
        let mut ctx = self.ctx();
        ctx.board_points.insert(board_id, field);
        ctx.touch_board(board_id);
        Ok(cfg)
    }

    /// Champs story points candidats, par priorité décroissante (sans doublon) : champ
    /// d'estimation du board (s'il est connu), puis catalogue global.
    fn points_candidates(&self, cat: &FieldCatalog, board: Option<u64>) -> Vec<String> {
        let board_field = board.and_then(|b| self.ctx().board_points.get(&b).cloned().flatten());
        let mut out: Vec<String> = Vec::new();
        for f in [
            board_field.as_ref(),
            cat.points_jsw.as_ref(),
            cat.points_legacy.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if !out.contains(f) {
                out.push(f.clone());
            }
        }
        out
    }

    /// Champ story points à utiliser en écriture pour le board donné (catalogue global sinon).
    async fn points_field_for_write(&self, board: Option<u64>) -> Result<String> {
        let cat = self.catalog().await?;
        self.points_candidates(cat, board)
            .into_iter()
            .next()
            .ok_or_else(|| {
                Error::Invalid("aucun champ « story points » trouvé sur cette instance Jira".into())
            })
    }

    /// Mémorise les issues écrites (par clé) pour `reconcileIssues`. Une clé jamais lue (id
    /// inconnu) est ignorée.
    fn note_written_keys<'a>(&self, keys: impl IntoIterator<Item = &'a String>) {
        let window = self.reconcile_window;
        let mut ctx = self.ctx();
        for key in keys {
            match ctx.issue_ids.get(&key.trim().to_ascii_uppercase()).copied() {
                Some(id) => ctx.note_written(id, window),
                None => tracing::debug!(
                    key = key.as_str(),
                    "id d'issue inconnu : pas de réconciliation"
                ),
            }
        }
    }

    // --- Recherche JQL ------------------------------------------------------------------------

    /// Recherche JQL ; `board` : board du contexte de lecture (champ d'estimation, mémorisation).
    async fn search(&self, jql: &str, board: Option<u64>) -> Result<Vec<Issue>> {
        let cat = self.catalog().await?.clone();
        let points = self.points_candidates(&cat, board);
        let mut fields: Vec<String> = [
            "summary",
            "description",
            "status",
            "issuetype",
            "priority",
            "assignee",
            "fixVersions",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
        fields.extend(cat.sprint.iter().cloned());
        fields.extend(points.iter().cloned());
        let reconcile = {
            let window = self.reconcile_window;
            self.ctx().reconcile_ids(window)
        };

        let url = self.endpoint(&["rest", "api", "3", "search", "jql"])?;
        let mut restarted = false;
        'restart: loop {
            let mut out = Vec::new();
            let mut token: Option<String> = None;
            for page_no in 0..MAX_PAGES {
                let mut body = json!({
                    "jql": jql,
                    "fields": fields,
                    "maxResults": SEARCH_PAGE_SIZE,
                    "fieldsByKeys": false,
                });
                if let Some(t) = &token {
                    body["nextPageToken"] = json!(t);
                }
                if !reconcile.is_empty() {
                    body["reconcileIssues"] = json!(reconcile);
                }
                // Lecture : nouvel essai autorisé malgré le POST.
                let res = self
                    .send(Method::POST, url.clone(), Some(&body), true)
                    .await;
                let text = match res {
                    Ok((_, text)) => text,
                    // Jeton de page expiré : recommencer une fois depuis le début.
                    Err(Error::Api { status: 400, .. }) if page_no > 0 && !restarted => {
                        tracing::warn!("search/jql : nextPageToken refusé, reprise du début");
                        restarted = true;
                        continue 'restart;
                    }
                    Err(e) => return Err(e),
                };
                let page: SearchPage = decode(&text, url.path())?;
                for raw in page.issues {
                    out.push(parse_issue(raw, cat.sprint.as_deref(), &points));
                }
                let next = page.next_page_token.filter(|t| !t.is_empty());
                match next {
                    Some(t) if page.is_last != Some(true) => {
                        if token.as_deref() == Some(t.as_str()) {
                            return Err(Error::Decode(
                                "search/jql : nextPageToken identique d'une page à l'autre".into(),
                            ));
                        }
                        token = Some(t);
                    }
                    _ => {
                        let mut ctx = self.ctx();
                        if let Some(b) = board {
                            ctx.touch_board(b);
                        }
                        for issue in &out {
                            ctx.note_issue(&issue.key, issue.id.trim().parse().ok(), board);
                        }
                        return Ok(out);
                    }
                }
            }
            return Err(Error::Decode(format!(
                "search/jql : pagination interrompue après {MAX_PAGES} pages"
            )));
        }
    }

    // --- Écritures agile par lots -------------------------------------------------------------

    async fn post_in_batches(&self, segments: &[&str], issue_keys: &[String]) -> Result<()> {
        let url = self.endpoint(segments)?;
        self.note_written_keys(issue_keys);
        let batches = issue_keys.chunks(WRITE_BATCH).count();
        for (i, chunk) in issue_keys.chunks(WRITE_BATCH).enumerate() {
            if let Err(e) = self
                .write_no_content(Method::POST, url.clone(), &json!({ "issues": chunk }))
                .await
            {
                if i > 0 {
                    tracing::warn!(
                        lot = i + 1,
                        lots = batches,
                        "échec d'un lot : les lots précédents sont déjà appliqués"
                    );
                }
                return Err(e);
            }
        }
        Ok(())
    }
}

impl FieldCatalog {
    fn from_fields(fields: &[FieldDto]) -> Self {
        fn custom(f: &FieldDto) -> Option<&str> {
            f.schema.as_ref().and_then(|s| s.custom.as_deref())
        }
        let named = |f: &FieldDto, name: &str| {
            f.name.eq_ignore_ascii_case(name)
                || f.untranslated_name
                    .as_deref()
                    .is_some_and(|n| n.eq_ignore_ascii_case(name))
        };

        let sprints: Vec<&FieldDto> = fields
            .iter()
            .filter(|f| custom(f) == Some(SPRINT_SCHEMA))
            .collect();
        if sprints.len() > 1 {
            tracing::warn!(
                count = sprints.len(),
                "plusieurs champs Sprint : celui nommé « Sprint » (sinon le premier) est utilisé"
            );
        }
        let sprint = sprints
            .iter()
            .find(|f| named(f, "Sprint"))
            .or(sprints.first())
            .map(|f| f.id.clone());

        let points_jsw = fields
            .iter()
            .find(|f| custom(f) == Some(JSW_POINTS_SCHEMA))
            .map(|f| f.id.clone());
        let points_legacy = fields
            .iter()
            .find(|f| custom(f) == Some(FLOAT_SCHEMA) && named(f, "Story Points"))
            .map(|f| f.id.clone());
        Self {
            sprint,
            points_jsw,
            points_legacy,
        }
    }
}

fn decode<T: DeserializeOwned>(text: &str, ctx: &str) -> Result<T> {
    if text.trim().is_empty() {
        return Err(Error::Decode(format!("{ctx} : corps de réponse vide")));
    }
    serde_json::from_str(text).map_err(|e| Error::Decode(format!("{ctx} : {e}")))
}

/// `Retry-After` : nombre de secondes ou HTTP-date (IMF-fixdate).
fn parse_retry_after(value: &str) -> Option<Duration> {
    let value = value.trim();
    if let Ok(secs) = value.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    let at = timeutil::parse_http_date(value)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default();
    Some(Duration::from_secs(at.saturating_sub(now).max(0) as u64))
}

/// `true` si la partie autorité d'une URL brute contient des identifiants (`user:pass@`).
/// Analyse textuelle volontairement large : appelée avant toute autre validation.
fn raw_url_has_userinfo(raw: &str) -> bool {
    let raw = raw.trim();
    let rest = match raw.split_once(':') {
        Some((scheme, rest))
            if !scheme.is_empty()
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) =>
        {
            rest
        }
        _ => raw,
    };
    let rest = rest.trim_start_matches(['/', '\\']);
    rest.split(['/', '\\', '?', '#'])
        .next()
        .is_some_and(|authority| authority.contains('@'))
}

/// `schéma://hôte[:port]` (sans chemin ni requête) : seule forme d'URL affichée dans les erreurs.
fn url_origin(url: &Url) -> String {
    let host = url.host_str().unwrap_or("?");
    match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    }
}

/// Cible d'une redirection pour le message d'erreur : hôte seul si `Location` est absolue, chemin
/// (sans requête) si elle est relative.
fn redirect_target(location: &str) -> String {
    match Url::parse(location.trim()) {
        Ok(u) => u.host_str().unwrap_or("?").to_owned(),
        Err(_) => {
            let path = location.trim().split(['?', '#']).next().unwrap_or("");
            one_line(path)
        }
    }
}

/// Guillemets JQL (`"…"`) avec échappement de `\` et `"`.
fn jql_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Identifiant numérique Jira (version, filtre…) : refuse tout le reste (évite une injection JQL).
fn numeric_id(id: &str, what: &str) -> Result<u64> {
    let id = id.trim();
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Error::Invalid(format!("{what} invalide : « {id} »")));
    }
    id.parse()
        .map_err(|_| Error::Invalid(format!("{what} invalide : « {id} »")))
}

/// Corps d'erreur standard Jira : `{"errorMessages": [...], "errors": {"champ": "message"}}`.
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ApiErrorBody {
    #[serde(default)]
    error_messages: Vec<String>,
    #[serde(default)]
    errors: BTreeMap<String, String>,
}

/// Extrait d'un corps non structuré : une ligne, espaces normalisés, 200 caractères max.
fn one_line(body: &str) -> String {
    let normalized = body.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = normalized.chars().take(200).collect();
    if normalized.chars().count() > 200 {
        out.push('…');
    }
    out
}

fn error_from_response(
    status: StatusCode,
    retry_after: Option<Duration>,
    content_type: Option<&str>,
    location: Option<&str>,
    body: &str,
) -> Error {
    let reason = status.canonical_reason().unwrap_or("").to_owned();
    let parsed: ApiErrorBody = serde_json::from_str(body).unwrap_or_default();
    let mut parts = parsed.error_messages;
    parts.extend(parsed.errors.into_iter().map(|(k, v)| format!("{k} : {v}")));
    let textual = match content_type {
        None => true,
        Some(ct) => ct.contains("json") || ct.starts_with("text/plain"),
    };
    let message = if !parts.is_empty() {
        one_line(&parts.join(" ; "))
    } else if textual && !body.trim().is_empty() {
        one_line(body)
    } else {
        reason
    };

    match status {
        StatusCode::UNAUTHORIZED => Error::Unauthorized,
        StatusCode::FORBIDDEN => Error::Forbidden(message),
        StatusCode::NOT_FOUND => Error::NotFound(message),
        StatusCode::TOO_MANY_REQUESTS => Error::RateLimited { retry_after },
        s if s.is_redirection() => Error::Api {
            status: s.as_u16(),
            message: match location.map(redirect_target).filter(|t| !t.is_empty()) {
                Some(target) => {
                    format!("redirection refusée vers « {target} » (vérifier l'URL de base Jira)")
                }
                None => "redirection refusée (vérifier l'URL de base Jira)".into(),
            },
        },
        s => Error::Api {
            status: s.as_u16(),
            message,
        },
    }
}

// --- DTO ---------------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OffsetPage<T> {
    #[serde(default = "Vec::new", alias = "issueTypes")]
    values: Vec<T>,
    is_last: Option<bool>,
    total: Option<u64>,
    start_at: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchPage {
    #[serde(default)]
    issues: Vec<IssueDto>,
    next_page_token: Option<String>,
    is_last: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FieldDto {
    id: String,
    #[serde(default)]
    name: String,
    untranslated_name: Option<String>,
    schema: Option<FieldSchemaDto>,
}

#[derive(Deserialize)]
struct FieldSchemaDto {
    custom: Option<String>,
}

#[derive(Deserialize, Default)]
struct BoardConfigDto {
    filter: Option<IdDto>,
    estimation: Option<EstimationDto>,
}

impl BoardConfigDto {
    fn estimation_field(&self) -> Option<&str> {
        let est = self.estimation.as_ref()?;
        if est.kind.as_deref() != Some("field") {
            return None;
        }
        est.field.as_ref()?.field_id.as_deref()
    }
}

#[derive(Deserialize)]
struct IdDto {
    id: Value,
}

#[derive(Deserialize)]
struct EstimationDto {
    #[serde(rename = "type")]
    kind: Option<String>,
    field: Option<EstimationFieldDto>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EstimationFieldDto {
    field_id: Option<String>,
}

/// Id tantôt string, tantôt entier selon l'API.
fn id_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn non_empty(s: Option<String>) -> Option<String> {
    s.filter(|s| !s.trim().is_empty())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserDto {
    #[serde(default)]
    account_id: String,
    display_name: Option<String>,
    email_address: Option<String>,
}

/// Nom affiché d'un utilisateur dont Jira ne renvoie pas `displayName` (compte supprimé, profil
/// restreint).
const UNKNOWN_USER: &str = "(utilisateur inconnu)";

impl From<UserDto> for User {
    fn from(d: UserDto) -> Self {
        Self {
            account_id: d.account_id,
            display_name: non_empty(d.display_name).unwrap_or_else(|| UNKNOWN_USER.into()),
            email: d.email_address,
        }
    }
}

#[derive(Deserialize)]
struct ProjectDto {
    id: String,
    key: String,
    name: String,
}

#[derive(Deserialize)]
struct BoardDto {
    id: u64,
    name: String,
    #[serde(rename = "type")]
    kind: Option<String>,
    location: Option<BoardLocationDto>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BoardLocationDto {
    project_key: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SprintDto {
    id: u64,
    name: String,
    state: String,
    goal: Option<String>,
    start_date: Option<String>,
    end_date: Option<String>,
    origin_board_id: Option<u64>,
}

fn sprint_state(s: &str) -> Option<SprintState> {
    match s.to_ascii_lowercase().as_str() {
        "active" => Some(SprintState::Active),
        "future" => Some(SprintState::Future),
        "closed" => Some(SprintState::Closed),
        _ => None,
    }
}

impl TryFrom<SprintDto> for Sprint {
    type Error = Error;
    fn try_from(d: SprintDto) -> Result<Self> {
        Ok(Self {
            id: d.id,
            board_id: d.origin_board_id,
            state: sprint_state(&d.state)
                .ok_or_else(|| Error::Decode(format!("état de sprint inconnu : {}", d.state)))?,
            name: d.name,
            goal: non_empty(d.goal),
            start_date: d.start_date,
            end_date: d.end_date,
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StatusCategoryDto {
    key: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StatusDto {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    status_category: Option<StatusCategoryDto>,
}

/// Libellé de repli d'un statut ou d'un type d'issue absent ou illisible.
const UNKNOWN_LABEL: &str = "(inconnu)";

impl From<StatusDto> for Status {
    fn from(d: StatusDto) -> Self {
        let category = match d.status_category.as_ref().map(|c| c.key.as_str()) {
            Some("done") => StatusCategory::Done,
            Some("indeterminate") => StatusCategory::InProgress,
            _ => StatusCategory::ToDo,
        };
        Self {
            id: d.id,
            name: non_empty(Some(d.name)).unwrap_or_else(|| UNKNOWN_LABEL.into()),
            category,
        }
    }
}

#[derive(Deserialize)]
struct IssueTypeDto {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    subtask: bool,
}

impl From<IssueTypeDto> for IssueType {
    fn from(d: IssueTypeDto) -> Self {
        Self {
            id: d.id,
            name: non_empty(Some(d.name)).unwrap_or_else(|| UNKNOWN_LABEL.into()),
            subtask: d.subtask,
        }
    }
}

#[derive(Deserialize)]
struct NamedDto {
    name: String,
}

#[derive(Deserialize)]
struct VersionRefDto {
    id: String,
    name: String,
}

#[derive(Deserialize)]
struct IssueDto {
    id: String,
    key: String,
    #[serde(default)]
    fields: Map<String, Value>,
}

/// Lit un champ connu de l'issue ; `null`/absent -> `None`. Valeur illisible : `None` + warn
/// (l'issue est dégradée plutôt que de faire échouer tout le chargement).
fn field<T: DeserializeOwned>(fields: &mut Map<String, Value>, name: &str, key: &str) -> Option<T> {
    match fields.remove(name) {
        None | Some(Value::Null) => None,
        Some(v) => match serde_json::from_value(v) {
            Ok(t) => Some(t),
            Err(e) => {
                tracing::warn!(issue = key, champ = name, error = %e, "champ d'issue illisible (ignoré)");
                None
            }
        },
    }
}

/// Sprint courant : entrée `active` du champ Sprint, sinon la première `future`.
fn current_sprint(value: Option<&Value>) -> Option<u64> {
    let entries = value?.as_array()?;
    let pick = |wanted: &str| {
        entries.iter().find_map(|s| {
            let state = s.get("state")?.as_str()?;
            if state.eq_ignore_ascii_case(wanted) {
                s.get("id")?.as_u64()
            } else {
                None
            }
        })
    };
    pick("active").or_else(|| pick("future"))
}

/// Convertit une issue de `search/jql`. Ne échoue jamais : un champ attendu absent ou illisible
/// (statut, type, utilisateur sans nom…) prend une valeur par défaut, avec un warn.
fn parse_issue(dto: IssueDto, sprint_field: Option<&str>, points_fields: &[String]) -> Issue {
    let key = dto.key;
    let mut f = dto.fields;
    let status: Status = match field::<StatusDto>(&mut f, "status", &key) {
        Some(s) => s.into(),
        None => {
            tracing::warn!(
                issue = key.as_str(),
                "issue sans statut : valeur par défaut"
            );
            Status {
                id: String::new(),
                name: UNKNOWN_LABEL.into(),
                category: StatusCategory::ToDo,
            }
        }
    };
    let issue_type: IssueType = match field::<IssueTypeDto>(&mut f, "issuetype", &key) {
        Some(t) => t.into(),
        None => {
            tracing::warn!(issue = key.as_str(), "issue sans type : valeur par défaut");
            IssueType {
                id: String::new(),
                name: UNKNOWN_LABEL.into(),
                subtask: false,
            }
        }
    };
    let description = f.remove("description").unwrap_or(Value::Null);
    let description_is_lossless = adf::is_lossless(&description);
    let description = match &description {
        Value::Null => None,
        // Format v2 (string wiki) toléré si jamais renvoyé.
        Value::String(s) => Some(s.clone()),
        doc => Some(adf::adf_to_text(doc)),
    }
    .filter(|s| !s.is_empty());
    let priority: Option<NamedDto> = field(&mut f, "priority", &key);
    let assignee: Option<UserDto> = field(&mut f, "assignee", &key);
    if assignee
        .as_ref()
        .is_some_and(|a| non_empty(a.display_name.clone()).is_none())
    {
        tracing::warn!(
            issue = key.as_str(),
            "assignee sans displayName : nom par défaut"
        );
    }
    let fix_versions: Option<Vec<VersionRefDto>> = field(&mut f, "fixVersions", &key);
    let story_points = points_fields
        .iter()
        .find_map(|name| f.get(name).and_then(Value::as_f64));
    let sprint_id = current_sprint(sprint_field.and_then(|s| f.get(s)));
    Issue {
        id: dto.id,
        summary: field::<String>(&mut f, "summary", &key).unwrap_or_default(),
        key,
        description,
        description_is_lossless,
        status,
        issue_type,
        priority: priority.map(|p| p.name),
        assignee: assignee.map(Into::into),
        story_points,
        fix_versions: fix_versions
            .unwrap_or_default()
            .into_iter()
            .map(|v| VersionRef {
                id: v.id,
                name: v.name,
            })
            .collect(),
        sprint_id,
    }
}

#[derive(Deserialize)]
struct TransitionsDto {
    #[serde(default)]
    transitions: Vec<TransitionDto>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TransitionDto {
    id: String,
    name: String,
    to: StatusDto,
    #[serde(default)]
    has_screen: bool,
    #[serde(default)]
    fields: BTreeMap<String, TransitionFieldDto>,
}

#[derive(Deserialize)]
struct TransitionFieldDto {
    #[serde(default)]
    required: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VersionDto {
    id: String,
    #[serde(rename = "self")]
    self_link: Option<String>,
    #[serde(default)]
    project_id: Value,
    name: String,
    description: Option<String>,
    #[serde(default)]
    released: bool,
    #[serde(default)]
    archived: bool,
    start_date: Option<String>,
    release_date: Option<String>,
}

/// `self` de la version cible, vérifié (URL http(s) absolue, même hôte que l'URL de base, se
/// terminant par l'id attendu).
fn version_self_link(dto: &VersionDto, expected_id: &str, base: &Url) -> Result<String> {
    let bad = |why: &str| Error::Decode(format!("version {expected_id} : {why}"));
    if dto.id.trim() != expected_id {
        return Err(bad("id inattendu dans la réponse"));
    }
    let raw = dto
        .self_link
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad("champ `self` absent"))?;
    let url = Url::parse(raw).map_err(|_| bad("champ `self` invalide"))?;
    let last = url
        .path_segments()
        .and_then(|mut segs| segs.rfind(|s| !s.is_empty()));
    if !matches!(url.scheme(), "http" | "https") || last != Some(expected_id) {
        return Err(bad("champ `self` inattendu"));
    }
    let same_host = url
        .host_str()
        .zip(base.host_str())
        .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b));
    if !same_host {
        return Err(bad(
            "champ `self` sur un autre hôte que l'instance configurée",
        ));
    }
    Ok(raw.to_owned())
}

impl From<VersionDto> for Version {
    fn from(d: VersionDto) -> Self {
        Self {
            id: d.id,
            project_id: id_string(&d.project_id).unwrap_or_default(),
            name: d.name,
            description: non_empty(d.description),
            released: d.released,
            archived: d.archived,
            start_date: d.start_date,
            release_date: d.release_date,
        }
    }
}

#[derive(Deserialize)]
struct CreatedIssueDto {
    id: Option<Value>,
    key: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RankResultDto {
    #[serde(default)]
    entries: Vec<RankEntryDto>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RankEntryDto {
    issue_key: Option<String>,
    issue_id: Option<u64>,
    status: Option<u16>,
    #[serde(default)]
    errors: Vec<String>,
}

// --- Implémentation du contrat ------------------------------------------------------------------

#[async_trait]
impl JiraApi for JiraHttp {
    async fn myself(&self) -> Result<User> {
        let dto: UserDto = self
            .get_json(self.endpoint(&["rest", "api", "3", "myself"])?)
            .await?;
        Ok(dto.into())
    }

    /// Projets Jira Software (`typeKey=software`) visibles.
    async fn projects(&self) -> Result<Vec<Project>> {
        let mut url = self.endpoint(&["rest", "api", "3", "project", "search"])?;
        url.query_pairs_mut()
            .append_pair("typeKey", "software")
            .append_pair("orderBy", "key");
        let dtos: Vec<ProjectDto> = self.get_all(url).await?;
        Ok(dtos
            .into_iter()
            .map(|d| Project {
                id: d.id,
                key: d.key,
                name: d.name,
            })
            .collect())
    }

    /// Boards `scrum` et `simple` (projets team-managed, dont le board unique est de type `simple`).
    async fn scrum_boards(&self, project_key: &str) -> Result<Vec<Board>> {
        // Le format multi-valeurs de `type` n'est pas documenté : deux appels, fusion par id.
        let mut dtos: Vec<BoardDto> = Vec::new();
        for kind in ["scrum", "simple"] {
            let mut url = self.endpoint(&["rest", "agile", "1.0", "board"])?;
            url.query_pairs_mut()
                .append_pair("type", kind)
                .append_pair("projectKeyOrId", project_key);
            match self.get_all::<BoardDto>(url).await {
                Ok(v) => {
                    for d in v {
                        if !dtos.iter().any(|x| x.id == d.id) {
                            dtos.push(d);
                        }
                    }
                }
                Err(Error::Api { status: 400, .. }) if kind == "simple" => {
                    tracing::warn!("boards `simple` refusés par Jira (400) : ignorés");
                }
                Err(e) => return Err(e),
            }
        }
        let boards: Vec<Board> = dtos
            .into_iter()
            .map(|d| Board {
                id: d.id,
                name: d.name,
                project_key: d.location.and_then(|l| l.project_key),
                board_type: d.kind.filter(|k| !k.trim().is_empty()),
            })
            .collect();
        let mut ctx = self.ctx();
        for b in &boards {
            if let Some(p) = &b.project_key {
                ctx.board_project
                    .insert(b.id, p.trim().to_ascii_uppercase());
            }
            if let Some(t) = &b.board_type {
                ctx.board_type.insert(b.id, t.to_ascii_lowercase());
            }
        }
        Ok(boards)
    }

    /// Un board sans sprints (board `simple` d'un projet team-managed sans la fonction Sprints) :
    /// 400 de Jira -> liste vide (warn). Un 404 n'est toléré que pour un board connu comme `simple`
    /// (sinon : board inexistant, `Error::NotFound`).
    async fn sprints(&self, board_id: u64, states: &[SprintState]) -> Result<Vec<Sprint>> {
        // Amorce (au mieux) le champ d'estimation du board, utile aux lectures d'issues qui suivent.
        let known = self.ctx().board_points.contains_key(&board_id);
        if !known && let Err(e) = self.board_config(board_id).await {
            tracing::debug!(error = %e, "configuration du board illisible (ignorée)");
        }
        self.ctx().touch_board(board_id);
        let id = board_id.to_string();
        let mut url = self.endpoint(&["rest", "agile", "1.0", "board", &id, "sprint"])?;
        if !states.is_empty() {
            let csv = states
                .iter()
                .map(|s| s.as_api_str())
                .collect::<Vec<_>>()
                .join(",");
            url.query_pairs_mut().append_pair("state", &csv);
        }
        let dtos: Vec<SprintDto> = match self.get_all(url).await {
            Ok(d) => d,
            Err(e) => {
                let simple = self
                    .ctx()
                    .board_type
                    .get(&board_id)
                    .is_some_and(|t| t == "simple");
                let tolerated = match &e {
                    Error::Api { status: 400, .. } => true,
                    Error::NotFound(_) => simple,
                    _ => false,
                };
                if !tolerated {
                    return Err(e);
                }
                tracing::warn!(board = board_id, error = %e, "board sans sprints : liste vide");
                Vec::new()
            }
        };
        let sprints: Vec<Sprint> = dtos
            .into_iter()
            .map(Sprint::try_from)
            .collect::<Result<_>>()?;
        let mut ctx = self.ctx();
        for s in &sprints {
            ctx.sprint_board.insert(s.id, board_id);
        }
        Ok(sprints)
    }

    async fn backlog_issues(&self, board_id: u64) -> Result<Vec<Issue>> {
        let cfg = self.board_config(board_id).await?;
        let filter_id = cfg
            .filter
            .as_ref()
            .and_then(|f| id_string(&f.id))
            .ok_or_else(|| Error::Decode(format!("board {board_id} : filtre absent")))?;
        let filter_id = numeric_id(&filter_id, "id de filtre")?;
        let jql = format!(
            "filter = {filter_id} AND (sprint is EMPTY OR (sprint not in openSprints() \
             AND sprint not in futureSprints())) AND statusCategory != Done \
             AND issuetype not in subtaskIssueTypes() ORDER BY Rank ASC"
        );
        self.search(&jql, Some(board_id)).await
    }

    async fn sprint_issues(&self, sprint_id: u64) -> Result<Vec<Issue>> {
        // Board connu si la liste des sprints d'un board l'a renvoyé (sinon catalogue global).
        let board = self.ctx().sprint_board.get(&sprint_id).copied();
        self.search(&format!("sprint = {sprint_id} ORDER BY Rank ASC"), board)
            .await
    }

    async fn move_to_sprint(&self, sprint_id: u64, issue_keys: &[String]) -> Result<()> {
        let id = sprint_id.to_string();
        self.post_in_batches(
            &["rest", "agile", "1.0", "sprint", &id, "issue"],
            issue_keys,
        )
        .await
    }

    async fn move_to_backlog(&self, issue_keys: &[String]) -> Result<()> {
        self.post_in_batches(&["rest", "agile", "1.0", "backlog", "issue"], issue_keys)
            .await
    }

    async fn rank(&self, issue_keys: &[String], position: &RankPosition) -> Result<()> {
        let anchor = match position {
            RankPosition::Before(k) | RankPosition::After(k) => k,
        };
        if issue_keys.contains(anchor) {
            return Err(Error::Invalid(
                "l'issue de référence fait partie des issues à déplacer".into(),
            ));
        }
        let url = self.endpoint(&["rest", "agile", "1.0", "issue", "rank"])?;
        self.note_written_keys(issue_keys);
        let mut previous_last: Option<&String> = None;
        for chunk in issue_keys.chunks(WRITE_BATCH) {
            // Lots suivants : placés après le dernier élément du lot précédent (ordre conservé).
            let body = match (previous_last, position) {
                (Some(last), _) => json!({ "issues": chunk, "rankAfterIssue": last }),
                (None, RankPosition::Before(k)) => json!({ "issues": chunk, "rankBeforeIssue": k }),
                (None, RankPosition::After(k)) => json!({ "issues": chunk, "rankAfterIssue": k }),
            };
            let (status, text) = self
                .send(Method::PUT, url.clone(), Some(&body), false)
                .await?;
            if status == StatusCode::MULTI_STATUS {
                let res: RankResultDto = decode(&text, url.path())?;
                let failed: Vec<String> = res
                    .entries
                    .into_iter()
                    .filter(|e| !e.status.is_some_and(|s| (200..300).contains(&s)))
                    .map(|e| {
                        let who = e
                            .issue_key
                            .or(e.issue_id.map(|i| i.to_string()))
                            .unwrap_or_else(|| "?".into());
                        let why = if e.errors.is_empty() {
                            e.status.map(|s| s.to_string()).unwrap_or_default()
                        } else {
                            e.errors.join(" ; ")
                        };
                        format!("{who} ({why})")
                    })
                    .collect();
                if !failed.is_empty() {
                    return Err(Error::Api {
                        status: 207,
                        message: one_line(&format!(
                            "rang non appliqué pour : {}",
                            failed.join(", ")
                        )),
                    });
                }
            }
            previous_last = chunk.last();
        }
        Ok(())
    }

    async fn start_sprint(&self, sprint_id: u64, params: &StartSprint) -> Result<Sprint> {
        let id = sprint_id.to_string();
        let url = self.endpoint(&["rest", "agile", "1.0", "sprint", &id])?;
        let mut body = json!({
            "state": "active",
            "startDate": params.start_date,
            "endDate": params.end_date,
        });
        if let Some(goal) = &params.goal {
            body["goal"] = json!(goal);
        }
        let dto: SprintDto = self.write_json(Method::POST, url, &body).await?;
        dto.try_into()
    }

    async fn close_sprint(&self, sprint_id: u64) -> Result<Sprint> {
        let id = sprint_id.to_string();
        let url = self.endpoint(&["rest", "agile", "1.0", "sprint", &id])?;
        let dto: SprintDto = self
            .write_json(Method::POST, url, &json!({ "state": "closed" }))
            .await?;
        dto.try_into()
    }

    async fn create_sprint(
        &self,
        board_id: u64,
        name: &str,
        goal: Option<String>,
    ) -> Result<Sprint> {
        if name.trim().is_empty() {
            return Err(Error::Invalid("le nom du sprint est obligatoire".into()));
        }
        let url = self.endpoint(&["rest", "agile", "1.0", "sprint"])?;
        let mut body = json!({ "name": name, "originBoardId": board_id });
        if let Some(goal) = goal {
            body["goal"] = json!(goal);
        }
        let dto: SprintDto = self.write_json(Method::POST, url, &body).await?;
        dto.try_into()
    }

    async fn update_sprint(&self, sprint_id: u64, update: &SprintUpdate) -> Result<Sprint> {
        let id = sprint_id.to_string();
        let url = self.endpoint(&["rest", "agile", "1.0", "sprint", &id])?;
        let mut body = Map::new();
        if let Some(name) = &update.name {
            if name.trim().is_empty() {
                return Err(Error::Invalid("le nom du sprint est obligatoire".into()));
            }
            body.insert("name".into(), json!(name));
        }
        if let Some(goal) = &update.goal {
            // Objectif vidé : chaîne vide (sémantique de `null` en POST partiel non documentée).
            body.insert("goal".into(), json!(goal.clone().unwrap_or_default()));
        }
        if let Some(d) = &update.start_date {
            body.insert("startDate".into(), json!(d));
        }
        if let Some(d) = &update.end_date {
            body.insert("endDate".into(), json!(d));
        }
        let dto: SprintDto = if body.is_empty() {
            self.get_json(url).await?
        } else {
            self.write_json(Method::POST, url, &Value::Object(body))
                .await?
        };
        let sprint: Sprint = dto.try_into()?;
        timeutil::check_closed_sprint_dates(update, &sprint)?;
        Ok(sprint)
    }

    async fn issue_types(&self, project_key: &str) -> Result<Vec<IssueType>> {
        let url = self.endpoint(&[
            "rest",
            "api",
            "3",
            "issue",
            "createmeta",
            project_key,
            "issuetypes",
        ])?;
        let dtos: Vec<IssueTypeDto> = self.get_all(url).await?;
        Ok(dtos.into_iter().map(Into::into).collect())
    }

    async fn create_issue(&self, issue: &NewIssue) -> Result<String> {
        if issue.summary.trim().is_empty() {
            return Err(Error::Invalid("le résumé est obligatoire".into()));
        }
        let mut fields = Map::new();
        fields.insert("project".into(), json!({ "key": issue.project_key }));
        fields.insert("issuetype".into(), json!({ "id": issue.issue_type_id }));
        fields.insert("summary".into(), json!(issue.summary));
        if let Some(doc) = issue.description.as_deref().and_then(adf::text_to_adf) {
            fields.insert("description".into(), doc);
        }
        if !issue.fix_version_ids.is_empty() {
            let versions: Vec<Value> = issue
                .fix_version_ids
                .iter()
                .map(|id| json!({ "id": id }))
                .collect();
            fields.insert("fixVersions".into(), Value::Array(versions));
        }
        // Champ du dernier board lu pour ce projet (la TUI crée depuis le board affiché) ; sinon
        // catalogue global.
        let board = self.ctx().board_for_project(&issue.project_key);
        if let Some(points) = issue.story_points {
            fields.insert(self.points_field_for_write(board).await?, json!(points));
        }
        let url = self.endpoint(&["rest", "api", "3", "issue"])?;
        let created: CreatedIssueDto = self
            .write_json(Method::POST, url, &json!({ "fields": fields }))
            .await?;
        let id = created
            .id
            .as_ref()
            .and_then(id_string)
            .and_then(|i| i.parse::<u64>().ok());
        let window = self.reconcile_window;
        let mut ctx = self.ctx();
        ctx.note_issue(&created.key, id, board);
        if let Some(id) = id {
            ctx.note_written(id, window);
        }
        Ok(created.key)
    }

    async fn update_issue(&self, key: &str, update: &IssueUpdate) -> Result<()> {
        let mut fields = Map::new();
        if let Some(summary) = &update.summary {
            if summary.trim().is_empty() {
                return Err(Error::Invalid("le résumé est obligatoire".into()));
            }
            fields.insert("summary".into(), json!(summary));
        }
        if let Some(desc) = &update.description {
            let doc = desc
                .as_deref()
                .and_then(adf::text_to_adf)
                .unwrap_or(Value::Null);
            fields.insert("description".into(), doc);
        }
        if let Some(points) = update.story_points {
            // Champ du board où l'issue a été lue en dernier, sinon du dernier board du projet.
            let board = self.ctx().board_for_issue(key);
            fields.insert(self.points_field_for_write(board).await?, json!(points));
        }
        let mut body = Map::new();
        if !fields.is_empty() {
            body.insert("fields".into(), Value::Object(fields));
        }
        if let Some(ids) = &update.fix_version_ids {
            let set: Vec<Value> = ids.iter().map(|id| json!({ "id": id })).collect();
            body.insert(
                "update".into(),
                json!({ "fixVersions": [ { "set": set } ] }),
            );
        }
        if body.is_empty() {
            return Ok(());
        }
        let url = self.endpoint(&["rest", "api", "3", "issue", key])?;
        self.note_written_keys([&key.to_owned()]);
        self.write_no_content(Method::PUT, url, &Value::Object(body))
            .await?;
        Ok(())
    }

    async fn transitions(&self, key: &str) -> Result<Vec<Transition>> {
        let mut url = self.endpoint(&["rest", "api", "3", "issue", key, "transitions"])?;
        url.query_pairs_mut()
            .append_pair("expand", "transitions.fields");
        let dto: TransitionsDto = self.get_json(url).await?;
        Ok(dto
            .transitions
            .into_iter()
            .map(|t| Transition {
                has_screen: t.has_screen || t.fields.values().any(|f| f.required),
                id: t.id,
                name: t.name,
                to: t.to.into(),
            })
            .collect())
    }

    async fn transition_issue(&self, key: &str, transition_id: &str) -> Result<()> {
        let url = self.endpoint(&["rest", "api", "3", "issue", key, "transitions"])?;
        self.note_written_keys([&key.to_owned()]);
        self.write_no_content(
            Method::POST,
            url,
            &json!({ "transition": { "id": transition_id } }),
        )
        .await?;
        Ok(())
    }

    async fn versions(&self, project_key: &str) -> Result<Vec<Version>> {
        let url = self.endpoint(&["rest", "api", "3", "project", project_key, "version"])?;
        let dtos: Vec<VersionDto> = self.get_all(url).await?;
        Ok(dtos.into_iter().map(Into::into).collect())
    }

    async fn version_issues(&self, project_key: &str, version_id: &str) -> Result<Vec<Issue>> {
        let version = numeric_id(version_id, "id de version")?;
        let jql = format!(
            "project = {} AND fixVersion = {version} ORDER BY Rank ASC",
            jql_quote(project_key)
        );
        let board = self.ctx().board_for_project(project_key);
        self.search(&jql, board).await
    }

    async fn create_version(&self, version: &NewVersion) -> Result<Version> {
        if version.name.trim().is_empty() {
            return Err(Error::Invalid(
                "le nom de la version est obligatoire".into(),
            ));
        }
        let project_id = numeric_id(&version.project_id, "id de projet")?;
        let mut body = json!({ "name": version.name, "projectId": project_id });
        if let Some(d) = &version.description {
            body["description"] = json!(d);
        }
        if let Some(d) = &version.start_date {
            body["startDate"] = json!(d);
        }
        if let Some(d) = &version.release_date {
            body["releaseDate"] = json!(d);
        }
        let url = self.endpoint(&["rest", "api", "3", "version"])?;
        let dto: VersionDto = self.write_json(Method::POST, url, &body).await?;
        Ok(dto.into())
    }

    async fn update_version(&self, version_id: &str, update: &VersionUpdate) -> Result<Version> {
        let url = self.endpoint(&["rest", "api", "3", "version", version_id])?;
        let mut body = Map::new();
        if let Some(name) = &update.name {
            if name.trim().is_empty() {
                return Err(Error::Invalid(
                    "le nom de la version est obligatoire".into(),
                ));
            }
            body.insert("name".into(), json!(name));
        }
        if let Some(d) = &update.description {
            body.insert("description".into(), json!(d.clone().unwrap_or_default()));
        }
        if let Some(d) = &update.start_date {
            body.insert("startDate".into(), json!(d));
        }
        if let Some(d) = &update.release_date {
            body.insert("releaseDate".into(), json!(d));
        }
        if let Some(r) = update.released {
            body.insert("released".into(), json!(r));
        }
        if let Some(a) = update.archived {
            body.insert("archived".into(), json!(a));
        }
        if let Some(target) = &update.move_unfixed_issues_to {
            let target_id = numeric_id(target, "version cible")?.to_string();
            if target_id == version_id.trim() {
                return Err(Error::Invalid(
                    "la version cible doit différer de la version modifiée".into(),
                ));
            }
            // Jira attend « the URL of the self link » de la version cible : on relit la cible et on
            // renvoie son `self` tel quel, après avoir vérifié qu'il désigne bien cette version sur
            // le même hôte que l'URL de base (jamais d'URL vers un hôte tiers).
            let target_url = self.endpoint(&["rest", "api", "3", "version", &target_id])?;
            let target: VersionDto = self.get_json(target_url).await?;
            let self_link = version_self_link(&target, &target_id, &self.base)?;
            body.insert("moveUnfixedIssuesTo".into(), json!(self_link));
        }
        let dto: VersionDto = if body.is_empty() {
            self.get_json(url).await?
        } else {
            self.write_json(Method::PUT, url, &Value::Object(body))
                .await?
        };
        Ok(dto.into())
    }

    async fn delete_version(
        &self,
        version_id: &str,
        move_fix_issues_to: Option<String>,
    ) -> Result<()> {
        let mut body = Map::new();
        if let Some(target) = &move_fix_issues_to {
            let target = numeric_id(target, "version cible")?;
            if target.to_string() == version_id.trim() {
                return Err(Error::Invalid(
                    "la version cible doit différer de la version supprimée".into(),
                ));
            }
            body.insert("moveFixIssuesTo".into(), json!(target));
        }
        let url = self.endpoint(&["rest", "api", "3", "version", version_id, "removeAndSwap"])?;
        self.write_no_content(Method::POST, url, &Value::Object(body))
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_formats() {
        assert_eq!(parse_retry_after("7"), Some(Duration::from_secs(7)));
        assert_eq!(parse_retry_after(" 0 "), Some(Duration::ZERO));
        // Date passée -> 0.
        assert_eq!(
            parse_retry_after("Sun, 06 Nov 1994 08:49:37 GMT"),
            Some(Duration::ZERO)
        );
        // Date future (~ +3600 s).
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let future = http_date(now + 3600);
        let d = parse_retry_after(&future).unwrap();
        assert!(d.as_secs() > 3500 && d.as_secs() <= 3600, "{d:?}");
        assert_eq!(parse_retry_after("bientôt"), None);
    }

    /// Formatage IMF-fixdate minimal pour les tests.
    fn http_date(secs: i64) -> String {
        let days = secs.div_euclid(86_400);
        let rem = secs.rem_euclid(86_400);
        // Inverse de days_from_civil (Hinnant).
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = yoe + era * 400 + i64::from(m <= 2);
        const M: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        format!(
            "Mon, {d:02} {} {y} {:02}:{:02}:{:02} GMT",
            M[(m - 1) as usize],
            rem / 3600,
            rem % 3600 / 60,
            rem % 60
        )
    }

    #[test]
    fn backoff_is_bounded_and_grows() {
        let p = RetryPolicy::default();
        let b1 = p.backoff(1);
        let b3 = p.backoff(3);
        assert!(b1 >= Duration::from_millis(1400) && b1 <= Duration::from_millis(2600));
        assert!(b3 >= Duration::from_millis(5600) && b3 <= Duration::from_millis(10_400));
        assert!(p.backoff(30) <= p.max_delay);
    }

    #[test]
    fn reconcile_list_is_bounded_deduplicated_and_expires() {
        let window = Duration::from_secs(60);
        let mut ctx = Context::default();
        for id in 1..=60 {
            ctx.note_written(id, window);
        }
        let ids = ctx.reconcile_ids(window);
        assert_eq!(ids.len(), RECONCILE_MAX);
        assert_eq!(ids.first(), Some(&11), "les plus anciennes sont évincées");
        ctx.note_written(20, window);
        let ids = ctx.reconcile_ids(window);
        assert_eq!(ids.len(), RECONCILE_MAX);
        assert_eq!(
            ids.last(),
            Some(&20),
            "réécriture : repasse en dernier, sans doublon"
        );
        assert_eq!(ids.iter().filter(|i| **i == 20).count(), 1);
        assert!(ctx.reconcile_ids(Duration::ZERO).is_empty(), "expirées");
    }

    #[test]
    fn issue_context_resolution() {
        let mut ctx = Context::default();
        ctx.board_project.insert(2, "OPS".into());
        ctx.note_issue("demo-1", Some(10000), Some(1));
        ctx.touch_board(2);
        assert_eq!(ctx.issue_ids.get("DEMO-1"), Some(&10000));
        assert_eq!(ctx.board_for_issue("DEMO-1"), Some(1));
        assert_eq!(
            ctx.board_for_issue("DEMO-7"),
            Some(1),
            "repli : board du projet"
        );
        assert_eq!(ctx.board_for_issue("OPS-7"), Some(2));
        assert_eq!(ctx.board_for_project("ops"), Some(2));
        assert_eq!(ctx.board_for_issue("NOPE-1"), None);
        assert_eq!(project_of("MY-PROJ-12").as_deref(), Some("MY-PROJ"));
        assert_eq!(project_of("12"), None);
    }

    #[test]
    fn userinfo_detection() {
        assert!(raw_url_has_userinfo("https://u:p@h"));
        assert!(raw_url_has_userinfo("https://u@h/x"));
        assert!(raw_url_has_userinfo("https:u:p@h"));
        assert!(raw_url_has_userinfo(" https:\\\\u:p@h"));
        assert!(!raw_url_has_userinfo("https://h/path@x"));
        assert!(!raw_url_has_userinfo("https://h?q=a@b"));
    }

    #[test]
    fn jql_quoting() {
        assert_eq!(jql_quote("DEMO"), "\"DEMO\"");
        assert_eq!(jql_quote(r#"A"B\C"#), r#""A\"B\\C""#);
        assert!(numeric_id("10100", "x").is_ok());
        assert!(numeric_id("1 OR 1=1", "x").is_err());
        assert!(numeric_id("", "x").is_err());
    }

    #[test]
    fn error_body_extracts() {
        let e = error_from_response(
            StatusCode::BAD_GATEWAY,
            None,
            Some("text/html; charset=utf-8"),
            None,
            "<html>\n<body>Proxy   error</body></html>",
        );
        assert!(matches!(e, Error::Api { status: 502, ref message } if message == "Bad Gateway"));
        let e = error_from_response(
            StatusCode::BAD_REQUEST,
            None,
            Some("text/plain"),
            None,
            "ligne 1\n\n   ligne   2",
        );
        assert!(matches!(e, Error::Api { ref message, .. } if message == "ligne 1 ligne 2"));
        let e = error_from_response(StatusCode::FOUND, None, None, None, "");
        assert!(matches!(e, Error::Api { status: 302, .. }));
        let e = error_from_response(
            StatusCode::MOVED_PERMANENTLY,
            None,
            None,
            Some("https://new-site.example.test/rest/api/3/myself?token=secret"),
            "",
        );
        assert!(
            matches!(e, Error::Api { status: 301, ref message }
                if message.contains("« new-site.example.test »") && !message.contains("secret")),
            "{e:?}"
        );
        let e = error_from_response(
            StatusCode::SEE_OTHER,
            None,
            None,
            Some("/rest/api/3/task/10?x=1"),
            "",
        );
        assert!(
            matches!(e, Error::Api { ref message, .. } if message.contains("« /rest/api/3/task/10 »")),
            "{e:?}"
        );
    }

    #[test]
    fn field_catalog_resolution() {
        let fields: Vec<FieldDto> =
            serde_json::from_str(include_str!("../tests/fixtures/auth/fields.json")).unwrap();
        let cat = FieldCatalog::from_fields(&fields);
        assert_eq!(cat.sprint.as_deref(), Some("customfield_10020"));
        assert_eq!(cat.points_jsw.as_deref(), Some("customfield_10016"));
        assert_eq!(cat.points_legacy.as_deref(), Some("customfield_10028"));
    }

    #[test]
    fn current_sprint_prefers_active_then_future() {
        let v = json!([
            {"id": 1, "state": "closed"},
            {"id": 3, "state": "future"},
            {"id": 2, "state": "active"}
        ]);
        assert_eq!(current_sprint(Some(&v)), Some(2));
        let v = json!([{"id": 1, "state": "closed"}, {"id": 3, "state": "FUTURE"}]);
        assert_eq!(current_sprint(Some(&v)), Some(3));
        assert_eq!(
            current_sprint(Some(&json!([{"id": 1, "state": "closed"}]))),
            None
        );
        assert_eq!(current_sprint(None), None);
    }
}
