//! Vues principales : sélecteur, Backlog, Sprint actif, Versions.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, LineGauge, List, ListItem, ListState, Paragraph, Wrap};

use super::super::action::fmt_points;
use super::super::dates;
use super::super::state::*;
use super::{category_color, issue_line, selected_style};
use jira_client::model::StatusCategory;

fn border(focused: bool) -> Style {
    if focused {
        Style::new().fg(Color::Cyan)
    } else {
        Style::new().fg(Color::DarkGray)
    }
}

fn render_list(
    frame: &mut Frame,
    area: Rect,
    block: Block,
    items: Vec<ListItem>,
    selected: Option<usize>,
) {
    let list = List::new(items)
        .block(block)
        .highlight_style(selected_style(true))
        .highlight_symbol("› ");
    let mut state = ListState::default().with_selected(selected);
    frame.render_stateful_widget(list, area, &mut state);
}

fn loading_or(app: &App, frame: &mut Frame, area: Rect, title: &str) -> bool {
    if app.loaded {
        return false;
    }
    let text = if app.pending > 0 {
        "Chargement…"
    } else {
        "Aucune donnée (R pour recharger)"
    };
    frame.render_widget(
        Paragraph::new(text).block(Block::bordered().title(format!(" {title} "))),
        area,
    );
    true
}

pub fn picker(frame: &mut Frame, app: &App, area: Rect) {
    let p = &app.picker;
    let (title, items): (String, Vec<ListItem>) = match &p.project {
        None => (
            " Projets ".into(),
            p.projects
                .iter()
                .map(|pr| {
                    ListItem::new(Line::from(vec![
                        Span::styled(format!("{:<10}", pr.key), Style::new().fg(Color::Cyan)),
                        Span::raw(pr.name.clone()),
                    ]))
                })
                .collect(),
        ),
        Some(pr) => (
            format!(" Boards Scrum de {} ", pr.key),
            p.boards
                .iter()
                .map(|b| ListItem::new(format!("#{:<6} {}", b.id, b.name)))
                .collect(),
        ),
    };
    let block = Block::bordered().title(title).border_style(border(true));
    if p.loading {
        frame.render_widget(Paragraph::new("Chargement…").block(block), area);
        return;
    }
    let sel = (!items.is_empty()).then_some(p.cursor);
    render_list(frame, area, block, items, sel);
}

pub fn backlog(frame: &mut Frame, app: &App, area: Rect) {
    if loading_or(app, frame, area, "Backlog") {
        return;
    }
    let inner_width = area.width.saturating_sub(4);
    let header = |name: &str, n: usize, pts: f64, collapsed: Option<bool>| {
        let arrow = match collapsed {
            Some(true) => "▶ ",
            Some(false) => "▼ ",
            None => "  ",
        };
        ListItem::new(Line::from(vec![
            Span::styled(
                format!("{arrow}{name}"),
                Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {n} issue(s) · {} pts", fmt_points(Some(pts))),
                Style::new().fg(Color::DarkGray),
            ),
        ]))
    };
    let items: Vec<ListItem> = app
        .backlog_rows()
        .into_iter()
        .map(|row| match row {
            BacklogRow::SprintHeader(id) => {
                let issues = app.data.issues_in(Some(id));
                let name = app.data.sprint_name(id).unwrap_or("?");
                header(
                    name,
                    issues.len(),
                    points_sum(issues),
                    Some(app.backlog.collapsed.contains(&id)),
                )
            }
            BacklogRow::BacklogHeader => header(
                "Backlog",
                app.data.backlog.len(),
                points_sum(&app.data.backlog),
                None,
            ),
            BacklogRow::Issue { loc, index } => {
                let issue = &app.data.issues_in(loc)[index];
                let mut line = issue_line(issue, inner_width.saturating_sub(2), true);
                line.spans.insert(0, Span::raw("  "));
                ListItem::new(line)
            }
        })
        .collect();
    let total: f64 = points_sum(&app.data.backlog)
        + app
            .data
            .future
            .iter()
            .map(|s| points_sum(app.data.issues_in(Some(s.id))))
            .sum::<f64>();
    let title = format!(
        " Backlog — sprints futurs + backlog · {} pts ",
        fmt_points(Some(total))
    );
    let block = Block::bordered().title(title).border_style(border(true));
    render_list(frame, area, block, items, Some(app.backlog.cursor));
}

pub fn sprint(frame: &mut Frame, app: &App, area: Rect) {
    if loading_or(app, frame, area, "Sprint actif") {
        return;
    }
    let Some(sprint) = &app.data.active else {
        let mut text = if app.data.future.is_empty() {
            "Aucun sprint actif ni futur.".to_string()
        } else {
            "Aucun sprint actif. s : démarrer un sprint futur.".to_string()
        };
        if app.data.foreign_sprints > 0 {
            text.push_str(&format!(
                "\n{} sprint(s) d'un autre board ignoré(s).",
                app.data.foreign_sprints
            ));
        }
        frame.render_widget(
            Paragraph::new(text).block(Block::bordered().title(" Sprint actif ")),
            area,
        );
        return;
    };
    let [head, cols_area] =
        Layout::vertical([Constraint::Length(5), Constraint::Min(3)]).areas(area);

    let issues = app.data.issues_in(Some(sprint.id));
    let total = points_sum(issues);
    let done = points_sum(
        issues
            .iter()
            .filter(|i| i.status.category == StatusCategory::Done),
    );
    let dates_txt = match (&sprint.start_date, &sprint.end_date) {
        (Some(s), Some(e)) => {
            let left = dates::days_until(app.today, e)
                .map(|d| {
                    if d >= 0 {
                        format!(" (J-{d})")
                    } else {
                        format!(" (dépassé de {} j)", -d)
                    }
                })
                .unwrap_or_default();
            format!("{} → {}{left}", dates::short(s), dates::short(e))
        }
        _ => "dates non définies".into(),
    };
    let mut dates_txt = dates_txt;
    if !app.data.parallel_active.is_empty() {
        dates_txt.push_str(&format!(
            "  ⚠ {} autre(s) sprint(s) actif(s) en parallèle, non affiché(s) : clôture désactivée",
            app.data.parallel_active.len()
        ));
    }
    let block = Block::bordered()
        .title(format!(" {} ", sprint.name))
        .border_style(border(false));
    let inner = block.inner(head);
    frame.render_widget(block, head);
    let [l1, l2, l3] = Layout::vertical([Constraint::Length(1); 3]).areas(inner);
    frame.render_widget(Paragraph::new(dates_txt), l1);
    frame.render_widget(
        Paragraph::new(format!(
            "Objectif : {}",
            sprint.goal.as_deref().unwrap_or("(aucun)")
        ))
        .wrap(Wrap { trim: true }),
        l2,
    );
    let ratio = if total > 0.0 {
        (done / total).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let gauge = LineGauge::default()
        .filled_style(Style::new().fg(Color::Green))
        .label(format!(
            "{}/{} pts terminés ({:.0} %)",
            fmt_points(Some(done)),
            fmt_points(Some(total)),
            ratio * 100.0
        ))
        .ratio(ratio);
    frame.render_widget(gauge, l3);

    let columns = app.sprint_columns();
    let areas = Layout::horizontal([Constraint::Ratio(1, 3); 3]).split(cols_area);
    for (c, ((cat, title), issues)) in COLUMNS.iter().zip(columns.iter()).enumerate() {
        let focused = app.sprint.column == c;
        let width = areas[c].width.saturating_sub(4);
        let items: Vec<ListItem> = issues
            .iter()
            .map(|i| ListItem::new(issue_line(i, width, false)))
            .collect();
        let block = Block::bordered()
            .title(Line::from(vec![
                Span::styled(
                    format!(" {title} "),
                    Style::new().fg(category_color(*cat)).bold(),
                ),
                Span::raw(format!(
                    "{} · {} pts ",
                    issues.len(),
                    fmt_points(Some(points_sum(issues.iter().copied())))
                )),
            ]))
            .border_style(border(focused));
        let sel = (focused && !issues.is_empty()).then_some(app.sprint.rows[c]);
        render_list(frame, areas[c], block, items, sel);
    }
}

pub fn versions(frame: &mut Frame, app: &App, area: Rect) {
    if loading_or(app, frame, area, "Versions") {
        return;
    }
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(38), Constraint::Percentage(62)]).areas(area);
    let ordered = app.data.versions_ordered();
    let unreleased = ordered.iter().filter(|v| !v.released).count();

    let section = |t: &str| {
        ListItem::new(Span::styled(
            t.to_string(),
            Style::new().fg(Color::Cyan).bold(),
        ))
    };
    let mut items = vec![section("Non livrées")];
    let mut selected_row = None;
    for (i, v) in ordered.iter().enumerate() {
        if i == unreleased {
            items.push(section("Livrées"));
        }
        if i == app.versions.cursor {
            selected_row = Some(items.len());
        }
        let date = v
            .release_date
            .as_deref()
            .map(dates::short)
            .unwrap_or_else(|| "—".into());
        let mark = if v.released { "✓" } else { " " };
        items.push(ListItem::new(format!("  {mark} {:<14} {date}", v.name)));
    }
    if unreleased == ordered.len() {
        items.push(section("Livrées"));
    }
    let list_focus = app.versions.focus == VersionsFocus::List;
    let block = Block::bordered()
        .title(" Versions ")
        .border_style(border(list_focus));
    render_list(frame, left, block, items, selected_row);

    let Some(v) = app.selected_version() else {
        frame.render_widget(
            Paragraph::new("Aucune version.").block(Block::bordered()),
            right,
        );
        return;
    };
    let issues = &app.versions.issues;
    let loading = app.versions.issues_for.as_ref() != Some(&v.id);
    let done = issues
        .iter()
        .filter(|i| i.status.category == StatusCategory::Done)
        .count();
    let title = if loading {
        format!(" Issues de {} ", v.name)
    } else {
        format!(
            " Issues de {} — {} · {} terminée(s) · {} pts ",
            v.name,
            issues.len(),
            done,
            fmt_points(Some(points_sum(issues)))
        )
    };
    let block = Block::bordered()
        .title(title)
        .border_style(border(!list_focus));
    let [desc_area, list_area] = Layout::vertical([
        Constraint::Length(if v.description.is_some() { 2 } else { 0 }),
        Constraint::Min(3),
    ])
    .areas(right);
    if let Some(d) = &v.description {
        frame.render_widget(
            Paragraph::new(d.clone()).wrap(Wrap { trim: true }).italic(),
            desc_area,
        );
    }
    if loading {
        frame.render_widget(Paragraph::new("Chargement…").block(block), list_area);
        return;
    }
    let width = list_area.width.saturating_sub(4);
    let items: Vec<ListItem> = issues
        .iter()
        .map(|i| ListItem::new(issue_line(i, width, true)))
        .collect();
    let sel = (!list_focus && !issues.is_empty()).then_some(app.versions.issue_cursor);
    render_list(frame, list_area, block, items, sel);
}
