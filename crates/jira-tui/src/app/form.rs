//! Formulaires (création/édition d'issue et de version, démarrage de sprint).
//! Logique pure : saisie, navigation entre champs, validation et construction du [`WriteOp`].

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use jiff::civil::Date;
use jiff::tz::TimeZone;
use jira_client::model::*;

use super::action::{WriteOp, fmt_points};
use super::dates;
use super::text::TextInput;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceOpt {
    pub label: String,
    pub id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FieldKind {
    Text(TextInput),
    Multi(TextInput),
    Choice {
        options: Vec<ChoiceOpt>,
        selected: usize,
    },
    ReadOnly(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldId {
    Summary,
    IssueType,
    Points,
    Version,
    Description,
    Name,
    StartDate,
    EndDate,
    ReleaseDate,
    Goal,
    Sprint,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub id: FieldId,
    pub label: &'static str,
    pub kind: FieldKind,
}

impl Field {
    fn text(id: FieldId, label: &'static str, value: impl Into<String>) -> Self {
        Self {
            id,
            label,
            kind: FieldKind::Text(TextInput::new(value)),
        }
    }
    fn multi(id: FieldId, label: &'static str, value: impl Into<String>) -> Self {
        Self {
            id,
            label,
            kind: FieldKind::Multi(TextInput::new(value)),
        }
    }
    fn read_only(id: FieldId, label: &'static str, value: impl Into<String>) -> Self {
        Self {
            id,
            label,
            kind: FieldKind::ReadOnly(value.into()),
        }
    }
    fn choice(id: FieldId, label: &'static str, options: Vec<ChoiceOpt>, selected: usize) -> Self {
        Self {
            id,
            label,
            kind: FieldKind::Choice { options, selected },
        }
    }
    pub fn editable(&self) -> bool {
        !matches!(self.kind, FieldKind::ReadOnly(_))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum FormKind {
    CreateIssue { project_key: String },
    EditIssue(Box<Issue>),
    CreateVersion { project_id: String },
    EditVersion(Box<Version>),
    StartSprint(Box<Sprint>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Form {
    pub title: String,
    pub kind: FormKind,
    pub fields: Vec<Field>,
    pub focus: usize,
    pub error: Option<String>,
    /// Valeurs initiales (voir [`Form::is_modified`]).
    initial: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormOutcome {
    Continue,
    Cancel,
    Submit,
}

fn version_choices(versions: &[Version], current: Option<&VersionRef>) -> (Vec<ChoiceOpt>, usize) {
    let mut options = vec![ChoiceOpt {
        label: "(aucune)".into(),
        id: None,
    }];
    for v in versions.iter().filter(|v| !v.archived) {
        let is_current = current.is_some_and(|c| c.id == v.id);
        if !v.released || is_current {
            options.push(ChoiceOpt {
                label: v.name.clone(),
                id: Some(v.id.clone()),
            });
        }
    }
    // Version actuelle absente de la liste (archivée…) : la garder sélectionnable.
    if let Some(c) = current
        && !options
            .iter()
            .any(|o| o.id.as_deref() == Some(c.id.as_str()))
    {
        options.push(ChoiceOpt {
            label: c.name.clone(),
            id: Some(c.id.clone()),
        });
    }
    let selected = current
        .and_then(|c| {
            options
                .iter()
                .position(|o| o.id.as_deref() == Some(c.id.as_str()))
        })
        .unwrap_or(0);
    (options, selected)
}

fn opt_string(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// Description : on ne retire que les blancs de fin pour préserver la mise en forme.
fn opt_description(s: &str) -> Option<String> {
    let t = s.trim_end();
    (!t.trim().is_empty()).then(|| t.to_string())
}

/// Description d'origine normalisée comme la saisie : une description dont seuls les blancs
/// de fin diffèrent n'est pas considérée comme modifiée.
fn orig_description(d: &Option<String>) -> Option<String> {
    d.as_deref().and_then(opt_description)
}

pub fn parse_points(s: &str) -> Result<Option<f64>, String> {
    let t = s.trim().replace(',', ".");
    if t.is_empty() {
        return Ok(None);
    }
    match t.parse::<f64>() {
        Ok(p) if p.is_finite() && p >= 0.0 => Ok(Some(p)),
        _ => Err(format!("story points invalides « {} »", s.trim())),
    }
}

fn opt_date(s: &str) -> Result<Option<String>, String> {
    match opt_string(s) {
        None => Ok(None),
        Some(d) => dates::parse_date(&d).map(|d| Some(d.to_string())),
    }
}

/// Description éditable seulement si la conversion ADF → texte est sans perte ; sinon la
/// réécrire détruirait la mise en forme riche (tableaux, mentions…) : champ en lecture seule.
fn description_field(issue: &Issue) -> Field {
    if issue.description_is_lossless {
        Field::multi(
            FieldId::Description,
            "Description",
            issue.description.clone().unwrap_or_default(),
        )
    } else {
        Field::read_only(
            FieldId::Description,
            "Description",
            "(mise en forme riche : modifiable dans Jira uniquement)",
        )
    }
}

impl Form {
    fn new(title: impl Into<String>, kind: FormKind, fields: Vec<Field>) -> Self {
        let focus = fields.iter().position(Field::editable).unwrap_or(0);
        let mut form = Self {
            title: title.into(),
            kind,
            fields,
            focus,
            error: None,
            initial: Vec::new(),
        };
        form.initial = form.values();
        form
    }

    fn values(&self) -> Vec<String> {
        self.fields
            .iter()
            .map(|f| match &f.kind {
                FieldKind::Text(t) | FieldKind::Multi(t) => t.value().to_string(),
                FieldKind::Choice { selected, .. } => selected.to_string(),
                FieldKind::ReadOnly(_) => String::new(),
            })
            .collect()
    }

    /// Une valeur a changé depuis l'ouverture (quitter perdrait la saisie).
    pub fn is_modified(&self) -> bool {
        self.values() != self.initial
    }

    /// Texte collé dans le champ actif (jamais interprété comme des touches).
    pub fn paste(&mut self, text: &str) {
        let Some(field) = self.fields.get_mut(self.focus) else {
            return;
        };
        match &mut field.kind {
            FieldKind::Text(t) => t.insert_str(text, false),
            FieldKind::Multi(t) => t.insert_str(text, true),
            FieldKind::Choice { .. } | FieldKind::ReadOnly(_) => return,
        }
        self.error = None;
    }

    pub fn create_issue(project_key: &str, types: &[IssueType], versions: &[Version]) -> Self {
        let type_opts: Vec<ChoiceOpt> = types
            .iter()
            .filter(|t| !t.subtask)
            .map(|t| ChoiceOpt {
                label: t.name.clone(),
                id: Some(t.id.clone()),
            })
            .collect();
        let (vopts, vsel) = version_choices(versions, None);
        Self::new(
            format!("Nouvelle issue ({project_key})"),
            FormKind::CreateIssue {
                project_key: project_key.into(),
            },
            vec![
                Field::text(FieldId::Summary, "Résumé", ""),
                Field::choice(FieldId::IssueType, "Type", type_opts, 0),
                Field::text(FieldId::Points, "Story points", ""),
                Field::choice(FieldId::Version, "Version", vopts, vsel),
                Field::multi(FieldId::Description, "Description", ""),
            ],
        )
    }

    pub fn edit_issue(issue: &Issue, versions: &[Version]) -> Self {
        // Plusieurs fixVersions : le choix unique les écraserait toutes → lecture seule.
        let version_field = if issue.fix_versions.len() > 1 {
            let names: Vec<&str> = issue.fix_versions.iter().map(|v| v.name.as_str()).collect();
            Field::read_only(
                FieldId::Version,
                "Version",
                format!(
                    "{} (plusieurs versions : modifiables dans Jira uniquement)",
                    names.join(", ")
                ),
            )
        } else {
            let (vopts, vsel) = version_choices(versions, issue.fix_versions.first());
            Field::choice(FieldId::Version, "Version", vopts, vsel)
        };
        Self::new(
            format!("Modifier {}", issue.key),
            FormKind::EditIssue(Box::new(issue.clone())),
            vec![
                Field::text(FieldId::Summary, "Résumé", issue.summary.clone()),
                Field::read_only(FieldId::IssueType, "Type", issue.issue_type.name.clone()),
                Field::text(
                    FieldId::Points,
                    "Story points",
                    issue
                        .story_points
                        .map(|p| fmt_points(Some(p)))
                        .unwrap_or_default(),
                ),
                version_field,
                description_field(issue),
            ],
        )
    }

    pub fn create_version(project_id: &str) -> Self {
        Self::new(
            "Nouvelle version",
            FormKind::CreateVersion {
                project_id: project_id.into(),
            },
            vec![
                Field::text(FieldId::Name, "Nom", ""),
                Field::text(FieldId::StartDate, "Début (AAAA-MM-JJ)", ""),
                Field::text(FieldId::ReleaseDate, "Livraison (AAAA-MM-JJ)", ""),
                Field::multi(FieldId::Description, "Description", ""),
            ],
        )
    }

    pub fn edit_version(v: &Version) -> Self {
        Self::new(
            format!("Modifier la version {}", v.name),
            FormKind::EditVersion(Box::new(v.clone())),
            vec![
                Field::text(FieldId::Name, "Nom", v.name.clone()),
                Field::text(
                    FieldId::ReleaseDate,
                    "Livraison (AAAA-MM-JJ)",
                    v.release_date.clone().unwrap_or_default(),
                ),
                Field::multi(
                    FieldId::Description,
                    "Description",
                    v.description.clone().unwrap_or_default(),
                ),
            ],
        )
    }

    pub fn start_sprint(sprint: &Sprint, today: Date) -> Self {
        Self::new(
            "Démarrer un sprint",
            FormKind::StartSprint(Box::new(sprint.clone())),
            vec![
                Field::read_only(FieldId::Sprint, "Sprint", sprint.name.clone()),
                Field::text(FieldId::StartDate, "Début (AAAA-MM-JJ)", today.to_string()),
                Field::text(
                    FieldId::EndDate,
                    "Fin (AAAA-MM-JJ)",
                    dates::plus_days(today, 14).to_string(),
                ),
                Field::text(
                    FieldId::Goal,
                    "Objectif",
                    sprint.goal.clone().unwrap_or_default(),
                ),
            ],
        )
    }

    fn field(&self, id: FieldId) -> Option<&Field> {
        self.fields.iter().find(|f| f.id == id)
    }

    #[cfg(test)]
    pub fn field_mut(&mut self, id: FieldId) -> Option<&mut Field> {
        self.fields.iter_mut().find(|f| f.id == id)
    }

    /// Valeur texte d'un champ (vide si absent ou non textuel).
    pub fn text(&self, id: FieldId) -> &str {
        match self.field(id).map(|f| &f.kind) {
            Some(FieldKind::Text(t) | FieldKind::Multi(t)) => t.value(),
            _ => "",
        }
    }

    /// Libellé de l'option sélectionnée d'un champ de choix.
    fn choice_label(&self, id: FieldId) -> Option<&str> {
        match self.field(id).map(|f| &f.kind) {
            Some(FieldKind::Choice { options, selected }) => {
                options.get(*selected).map(|o| o.label.as_str())
            }
            _ => None,
        }
    }

    /// Identifiant de l'option sélectionnée d'un champ de choix.
    pub fn choice(&self, id: FieldId) -> Option<&str> {
        match self.field(id).map(|f| &f.kind) {
            Some(FieldKind::Choice { options, selected }) => {
                options.get(*selected).and_then(|o| o.id.as_deref())
            }
            _ => None,
        }
    }

    fn move_focus(&mut self, forward: bool) {
        let n = self.fields.len();
        for step in 1..=n {
            let i = if forward {
                (self.focus + step) % n
            } else {
                (self.focus + n - step) % n
            };
            if self.fields[i].editable() {
                self.focus = i;
                return;
            }
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> FormOutcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => return FormOutcome::Cancel,
            KeyCode::Char('s') if ctrl => return FormOutcome::Submit,
            KeyCode::Tab | KeyCode::Down => {
                self.move_focus(true);
                return FormOutcome::Continue;
            }
            KeyCode::BackTab | KeyCode::Up => {
                self.move_focus(false);
                return FormOutcome::Continue;
            }
            _ => {}
        }
        let Some(field) = self.fields.get_mut(self.focus) else {
            return FormOutcome::Continue;
        };
        match &mut field.kind {
            FieldKind::Text(t) => {
                if key.code == KeyCode::Enter {
                    return FormOutcome::Submit;
                }
                if t.handle_key(key, false) {
                    self.error = None;
                }
            }
            FieldKind::Multi(t) => {
                if t.handle_key(key, true) {
                    self.error = None;
                }
            }
            FieldKind::Choice { options, selected } => match key.code {
                KeyCode::Enter => return FormOutcome::Submit,
                KeyCode::Left | KeyCode::Char('h') if !options.is_empty() => {
                    *selected = (*selected + options.len() - 1) % options.len();
                }
                KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ') if !options.is_empty() => {
                    *selected = (*selected + 1) % options.len();
                }
                _ => {}
            },
            FieldKind::ReadOnly(_) => {
                if key.code == KeyCode::Enter {
                    return FormOutcome::Submit;
                }
            }
        }
        FormOutcome::Continue
    }

    /// Valide le formulaire et construit l'opération d'écriture correspondante.
    /// Les littéraux du modèle finissent par `..Default::default()` : robustes aux champs
    /// ajoutés plus tard côté client (contrat additif), d'où l'`allow`.
    #[allow(clippy::needless_update)]
    pub fn build_op(&self, tz: &TimeZone) -> Result<WriteOp, String> {
        match &self.kind {
            FormKind::CreateIssue { project_key } => {
                let summary =
                    opt_string(self.text(FieldId::Summary)).ok_or("le résumé est obligatoire")?;
                let issue_type_id = self
                    .choice(FieldId::IssueType)
                    .ok_or("aucun type d'issue disponible")?
                    .to_string();
                Ok(WriteOp::CreateIssue(NewIssue {
                    project_key: project_key.clone(),
                    issue_type_id,
                    summary,
                    description: opt_description(self.text(FieldId::Description)),
                    story_points: parse_points(self.text(FieldId::Points))?,
                    fix_version_ids: self
                        .choice(FieldId::Version)
                        .map(str::to_string)
                        .into_iter()
                        .collect(),
                    ..Default::default()
                }))
            }
            FormKind::EditIssue(orig) => {
                let summary =
                    opt_string(self.text(FieldId::Summary)).ok_or("le résumé est obligatoire")?;
                let points = parse_points(self.text(FieldId::Points))?;
                // Description en lecture seule (non convertible sans perte) : jamais envoyée.
                let orig_desc = orig_description(&orig.description);
                let description = match self.field(FieldId::Description) {
                    Some(f) if f.editable() => opt_description(self.text(FieldId::Description)),
                    _ => orig_desc.clone(),
                };
                // Version : seulement si le champ est un choix (une seule fixVersion à l'origine) ;
                // en lecture seule (plusieurs), `fix_version_ids` n'est jamais envoyé.
                let version_editable = self
                    .field(FieldId::Version)
                    .is_some_and(|f| matches!(f.kind, FieldKind::Choice { .. }));
                let version = self.choice(FieldId::Version).map(str::to_string);
                let orig_version = orig.fix_versions.first().map(|v| v.id.clone());
                let version_changed = version_editable && version != orig_version;
                let fix_version_names = version_changed.then(|| {
                    version
                        .is_some()
                        .then(|| {
                            self.choice_label(FieldId::Version)
                                .unwrap_or("?")
                                .to_string()
                        })
                        .into_iter()
                        .collect()
                });
                let update = IssueUpdate {
                    summary: (summary != orig.summary).then_some(summary),
                    description: (description != orig_desc).then_some(description),
                    story_points: (points != orig.story_points).then_some(points),
                    fix_version_ids: version_changed.then(|| version.into_iter().collect()),
                    ..Default::default()
                };
                if update == IssueUpdate::default() {
                    return Err("aucune modification".into());
                }
                Ok(WriteOp::UpdateIssue {
                    key: orig.key.clone(),
                    update,
                    fix_version_names,
                })
            }
            FormKind::CreateVersion { project_id } => {
                let name = opt_string(self.text(FieldId::Name)).ok_or("le nom est obligatoire")?;
                Ok(WriteOp::CreateVersion(NewVersion {
                    project_id: project_id.clone(),
                    name,
                    description: opt_description(self.text(FieldId::Description)),
                    start_date: opt_date(self.text(FieldId::StartDate))?,
                    release_date: opt_date(self.text(FieldId::ReleaseDate))?,
                    ..Default::default()
                }))
            }
            FormKind::EditVersion(orig) => {
                let name = opt_string(self.text(FieldId::Name)).ok_or("le nom est obligatoire")?;
                let release = opt_date(self.text(FieldId::ReleaseDate))?;
                let description = opt_description(self.text(FieldId::Description));
                let update = VersionUpdate {
                    name: (name != orig.name).then_some(name),
                    description: (description != orig_description(&orig.description))
                        .then_some(description),
                    release_date: (release != orig.release_date).then_some(release),
                    ..Default::default()
                };
                if update == VersionUpdate::default() {
                    return Err("aucune modification".into());
                }
                Ok(WriteOp::UpdateVersion {
                    version_id: orig.id.clone(),
                    version_name: orig.name.clone(),
                    update,
                })
            }
            FormKind::StartSprint(sprint) => {
                let start = dates::parse_date(self.text(FieldId::StartDate))?;
                let end = dates::parse_date(self.text(FieldId::EndDate))?;
                if end <= start {
                    return Err("la date de fin doit être après la date de début".into());
                }
                Ok(WriteOp::StartSprint {
                    sprint_id: sprint.id,
                    sprint_name: sprint.name.clone(),
                    params: StartSprint {
                        start_date: dates::sprint_datetime(start, 9, tz),
                        end_date: dates::sprint_datetime(end, 17, tz),
                        goal: opt_string(self.text(FieldId::Goal)),
                        ..Default::default()
                    },
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn type_str(form: &mut Form, s: &str) {
        for c in s.chars() {
            form.handle_key(key(KeyCode::Char(c)));
        }
    }

    fn types() -> Vec<IssueType> {
        vec![
            IssueType {
                id: "1".into(),
                name: "Story".into(),
                subtask: false,
            },
            IssueType {
                id: "2".into(),
                name: "Sous-tâche".into(),
                subtask: true,
            },
            IssueType {
                id: "3".into(),
                name: "Bug".into(),
                subtask: false,
            },
        ]
    }

    fn version(id: &str, name: &str, released: bool) -> Version {
        Version {
            id: id.into(),
            project_id: "p".into(),
            name: name.into(),
            description: None,
            released,
            archived: false,
            start_date: None,
            release_date: None,
        }
    }

    #[test]
    fn create_issue_validation_and_build() {
        let versions = [version("100", "1.0", true), version("101", "1.1", false)];
        let mut f = Form::create_issue("DEMO", &types(), &versions);
        assert_eq!(
            f.build_op(&TimeZone::UTC).unwrap_err(),
            "le résumé est obligatoire"
        );
        type_str(&mut f, "Nouvelle");
        f.handle_key(key(KeyCode::Tab)); // type
        f.handle_key(key(KeyCode::Right)); // Bug (la sous-tâche est exclue)
        f.handle_key(key(KeyCode::Tab)); // points
        type_str(&mut f, "2,5");
        f.handle_key(key(KeyCode::Tab)); // version
        f.handle_key(key(KeyCode::Right)); // 1.1 (1.0 livrée exclue)
        f.handle_key(key(KeyCode::Tab)); // description
        type_str(&mut f, "a");
        assert_eq!(f.handle_key(key(KeyCode::Enter)), FormOutcome::Continue);
        type_str(&mut f, "b  ");
        assert_eq!(
            f.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
            FormOutcome::Submit
        );
        let WriteOp::CreateIssue(n) = f.build_op(&TimeZone::UTC).unwrap() else {
            panic!()
        };
        assert_eq!(n.summary, "Nouvelle");
        assert_eq!(n.issue_type_id, "3");
        assert_eq!(n.story_points, Some(2.5));
        assert_eq!(n.fix_version_ids, vec!["101".to_string()]);
        assert_eq!(n.description.as_deref(), Some("a\nb"));
    }

    #[test]
    fn edit_issue_sends_only_changes() {
        let mut issue = crate::app::tests::mock_issue("DEMO-1");
        issue.key = "DEMO-1".into();
        issue.summary = "Titre".into();
        issue.description = None;
        issue.description_is_lossless = true;
        issue.issue_type = types()[0].clone();
        issue.story_points = Some(3.0);
        issue.fix_versions = vec![VersionRef {
            id: "100".into(),
            name: "1.0".into(),
        }];
        let versions = [version("100", "1.0", true), version("101", "1.1", false)];
        let mut f = Form::edit_issue(&issue, &versions);
        assert_eq!(
            f.choice(FieldId::Version),
            Some("100"),
            "version livrée actuelle conservée"
        );
        assert_eq!(
            f.build_op(&TimeZone::UTC).unwrap_err(),
            "aucune modification"
        );
        f.handle_key(key(KeyCode::Tab)); // le type est en lecture seule : focus sur points
        assert_eq!(f.fields[f.focus].id, FieldId::Points);
        f.handle_key(key(KeyCode::Backspace));
        type_str(&mut f, "5");
        let WriteOp::UpdateIssue { key: k, update, .. } = f.build_op(&TimeZone::UTC).unwrap()
        else {
            panic!()
        };
        assert_eq!(k, "DEMO-1");
        assert_eq!(
            update,
            IssueUpdate {
                story_points: Some(Some(5.0)),
                ..Default::default()
            }
        );
        f.handle_key(key(KeyCode::Backspace));
        type_str(&mut f, "x");
        assert!(
            f.build_op(&TimeZone::UTC)
                .unwrap_err()
                .contains("story points")
        );
    }

    #[test]
    fn lossy_description_is_read_only_and_never_sent() {
        let mut issue = crate::app::tests::mock_issue("DEMO-2");
        issue.description = Some("Tableau riche".into());
        issue.description_is_lossless = false;
        let mut f = Form::edit_issue(&issue, &[]);
        let desc = f
            .fields
            .iter()
            .find(|x| x.id == FieldId::Description)
            .unwrap();
        assert!(!desc.editable());
        type_str(&mut f, " bis");
        let WriteOp::UpdateIssue { update, .. } = f.build_op(&TimeZone::UTC).unwrap() else {
            panic!()
        };
        assert_eq!(update.description, None, "description jamais écrasée");
        assert!(update.summary.is_some());
    }

    #[test]
    fn trailing_blanks_of_description_are_not_a_change() {
        let mut issue = crate::app::tests::mock_issue("DEMO-8");
        issue.description = Some("Texte\n\n  ".into());
        issue.description_is_lossless = true;
        let mut f = Form::edit_issue(&issue, &[]);
        assert!(!f.is_modified());
        assert_eq!(
            f.build_op(&TimeZone::UTC).unwrap_err(),
            "aucune modification"
        );
        type_str(&mut f, "!");
        assert!(f.is_modified());
        let WriteOp::UpdateIssue { update, .. } = f.build_op(&TimeZone::UTC).unwrap() else {
            panic!()
        };
        assert_eq!(
            update.description, None,
            "description inchangée non envoyée"
        );

        let mut v = version("101", "1.1", false);
        v.description = Some("Notes \n".into());
        let f = Form::edit_version(&v);
        assert_eq!(
            f.build_op(&TimeZone::UTC).unwrap_err(),
            "aucune modification"
        );
    }

    #[test]
    fn several_fix_versions_are_read_only() {
        let mut issue = crate::app::tests::mock_issue("DEMO-8");
        issue.fix_versions = vec![
            VersionRef {
                id: "101".into(),
                name: "1.1".into(),
            },
            VersionRef {
                id: "102".into(),
                name: "2.0".into(),
            },
        ];
        let versions = [version("101", "1.1", false), version("102", "2.0", false)];
        let mut f = Form::edit_issue(&issue, &versions);
        let field = f.field(FieldId::Version).unwrap();
        assert!(!field.editable());
        assert!(matches!(&field.kind, FieldKind::ReadOnly(t) if t.starts_with("1.1, 2.0")));
        type_str(&mut f, "!");
        let WriteOp::UpdateIssue {
            update,
            fix_version_names,
            ..
        } = f.build_op(&TimeZone::UTC).unwrap()
        else {
            panic!()
        };
        assert_eq!(update.fix_version_ids, None);
        assert_eq!(fix_version_names, None);

        // Une seule version : changement affiché par nom.
        issue.fix_versions.truncate(1);
        let mut f = Form::edit_issue(&issue, &versions);
        f.focus = f
            .fields
            .iter()
            .position(|x| x.id == FieldId::Version)
            .unwrap();
        f.handle_key(key(KeyCode::Right));
        let op = f.build_op(&TimeZone::UTC).unwrap();
        let WriteOp::UpdateIssue { update, .. } = &op else {
            panic!()
        };
        assert_eq!(update.fix_version_ids, Some(vec!["102".to_string()]));
        assert!(op.describe().1.contains(&"  version → 2.0".to_string()));
    }

    #[test]
    fn start_sprint_defaults_and_validation() {
        let sprint = Sprint {
            id: 11,
            board_id: Some(1),
            name: "S5".into(),
            state: SprintState::Future,
            goal: None,
            start_date: None,
            end_date: None,
        };
        let mut f = Form::start_sprint(&sprint, date(2026, 11, 11));
        assert_eq!(f.text(FieldId::StartDate), "2026-11-11");
        assert_eq!(f.text(FieldId::EndDate), "2026-11-25");
        let WriteOp::StartSprint { params, .. } = f.build_op(&TimeZone::UTC).unwrap() else {
            panic!()
        };
        assert_eq!(params.start_date, "2026-11-11T09:00:00.000+00:00");
        assert_eq!(params.end_date, "2026-11-25T17:00:00.000+00:00");
        assert_eq!(params.goal, None);
        if let Some(Field {
            kind: FieldKind::Text(t),
            ..
        }) = f.field_mut(FieldId::EndDate)
        {
            *t = TextInput::new("2026-11-01");
        }
        assert!(f.build_op(&TimeZone::UTC).unwrap_err().contains("après"));
    }

    #[test]
    fn version_forms() {
        let mut f = Form::create_version("10000");
        type_str(&mut f, "3.0.0");
        f.handle_key(key(KeyCode::Tab));
        type_str(&mut f, "bad");
        assert!(
            f.build_op(&TimeZone::UTC)
                .unwrap_err()
                .contains("date invalide")
        );
        let v = version("101", "1.1", false);
        let mut f = Form::edit_version(&v);
        f.handle_key(key(KeyCode::Tab));
        type_str(&mut f, "2027-01-15");
        let WriteOp::UpdateVersion { update, .. } = f.build_op(&TimeZone::UTC).unwrap() else {
            panic!()
        };
        assert_eq!(update.release_date, Some(Some("2027-01-15".into())));
        assert_eq!(update.name, None);
    }
}
