//! Rendu : fonctions pures de `&App` vers une `Frame`. Aucune mutation d'état.

mod popups;
mod views;

use jira_client::model::{Issue, StatusCategory};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Tabs};

use super::action::fmt_points;
use super::state::{App, Screen, Tab};

const SPINNER: [&str; 4] = ["|", "/", "-", "\\"];

pub fn draw(frame: &mut Frame, app: &App) {
    let [top, body, hints, status] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    draw_top(frame, app, top);
    match app.screen {
        Screen::Picker => views::picker(frame, app, body),
        Screen::Main => match app.tab {
            Tab::Backlog => views::backlog(frame, app, body),
            Tab::Sprint => views::sprint(frame, app, body),
            Tab::Versions => views::versions(frame, app, body),
        },
    }
    frame.render_widget(
        Paragraph::new(hint_line(app)).style(Style::new().fg(Color::DarkGray)),
        hints,
    );
    draw_status(frame, app, status);
    popups::draw(frame, app);
}

fn draw_top(frame: &mut Frame, app: &App, area: Rect) {
    let mut ctx = app
        .ctx
        .as_ref()
        .map(|c| format!(" {} · {} ", c.project.key, c.board.name))
        .unwrap_or_default();
    if app.read_only {
        ctx.push_str("[LECTURE SEULE] ");
    }
    let [left, right] = Layout::horizontal([
        Constraint::Min(10),
        Constraint::Length(ctx.chars().count() as u16),
    ])
    .areas(area);
    if app.screen == Screen::Main {
        let titles = Tab::ALL
            .iter()
            .map(|t| format!("{} {}", t.index() + 1, t.title()));
        let tabs = Tabs::new(titles).select(app.tab.index()).highlight_style(
            Style::new()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD | Modifier::REVERSED),
        );
        frame.render_widget(tabs, left);
    } else {
        frame.render_widget(
            Paragraph::new(" jira-rust — sélection du projet".bold()),
            left,
        );
    }
    frame.render_widget(Paragraph::new(ctx.cyan()), right);
}

fn hint_line(app: &App) -> &'static str {
    if app.screen == Screen::Picker {
        return " j/k naviguer · Entrée choisir · Esc retour · R recharger · ? aide · q quitter";
    }
    match app.tab {
        Tab::Backlog => {
            " j/k · Entrée détail/replier · t transition · m déplacer · K/J rang · c créer · e éditer · s démarrer sprint · ? aide"
        }
        Tab::Sprint => {
            " h/l colonne · j/k · Entrée détail · t transition · m déplacer · K/J rang · x clôturer · s démarrer · ? aide"
        }
        Tab::Versions => {
            " j/k · h/l liste/issues · Entrée détail · c créer · e éditer · r livrer · t transition · ? aide"
        }
    }
}

fn draw_status(frame: &mut Frame, app: &App, area: Rect) {
    let mut spans = Vec::new();
    if app.pending > 0 {
        spans.push(Span::styled(
            format!(" {} Chargement… ", SPINNER[app.spinner % SPINNER.len()]),
            Style::new().fg(Color::Yellow),
        ));
    }
    if !app.status.text.is_empty() {
        let style = if app.status.error {
            Style::new().fg(Color::White).bg(Color::Red)
        } else {
            Style::new().fg(Color::Green)
        };
        spans.push(Span::styled(format!(" {} ", app.status.text), style));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ---------------------------------------------------------------------- utilitaires partagés

pub(super) fn category_color(c: StatusCategory) -> Color {
    match c {
        StatusCategory::ToDo => Color::Gray,
        StatusCategory::InProgress => Color::Yellow,
        StatusCategory::Done => Color::Green,
    }
}

/// Tronque/complète à `width` caractères.
pub(super) fn fit(s: &str, width: usize) -> String {
    let n = s.chars().count();
    if n <= width {
        format!("{s:<width$}")
    } else if width == 0 {
        String::new()
    } else {
        let mut out: String = s.chars().take(width - 1).collect();
        out.push('…');
        out
    }
}

/// Ligne d'issue : clé, type, statut, résumé, points (alignés à droite).
pub(super) fn issue_line(issue: &Issue, width: u16, show_status: bool) -> Line<'static> {
    let width = width as usize;
    let key = fit(&issue.key, 10);
    let kind = fit(&issue.issue_type.name, 6);
    let status = if show_status {
        fit(&issue.status.name, 10)
    } else {
        String::new()
    };
    let pts = format!("{:>4}", fmt_points(issue.story_points));
    let fixed = 10 + 1 + 6 + 1 + if show_status { 11 } else { 0 } + 1 + 4;
    let summary = fit(&issue.summary, width.saturating_sub(fixed));
    let mut spans = vec![
        Span::styled(key, Style::new().fg(Color::Cyan)),
        Span::raw(" "),
        Span::styled(kind, Style::new().fg(Color::Magenta)),
        Span::raw(" "),
    ];
    if show_status {
        spans.push(Span::styled(
            status,
            Style::new().fg(category_color(issue.status.category)),
        ));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::raw(summary));
    spans.push(Span::raw(" "));
    spans.push(Span::styled(pts, Style::new().bold()));
    Line::from(spans)
}

pub(super) fn selected_style(active: bool) -> Style {
    if active {
        Style::new()
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::new()
    }
}

/// Rectangle centré (pourcentages de la zone, avec minimums absolus).
pub(super) fn centered(area: Rect, pct_w: u16, min_w: u16, height: u16) -> Rect {
    let w = (area.width * pct_w / 100).max(min_w).min(area.width);
    let h = height.min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}
