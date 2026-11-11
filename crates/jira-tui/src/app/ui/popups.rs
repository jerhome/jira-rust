//! Popups : aide, détail d'issue, listes de choix, formulaires, confirmation.

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Wrap};

use super::super::action::fmt_points;
use super::super::form::{FieldKind, Form};
use super::super::state::{App, Popup};
use super::{category_color, centered, selected_style};

pub const HELP: &[(&str, &str)] = &[
    ("Général", ""),
    (
        "1 / 2 / 3, Tab",
        "onglets Backlog / Sprint actif / Versions",
    ),
    ("j/k, ↑/↓, g/G", "naviguer, début/fin"),
    ("h/l, ←/→", "colonne (Sprint) ou liste/issues (Versions)"),
    (
        "Entrée",
        "détail de l'issue (replier un sprint dans le Backlog)",
    ),
    ("Espace", "replier/déplier le sprint futur (Backlog)"),
    ("p", "changer de projet / board"),
    ("R", "rafraîchir"),
    ("?", "cette aide"),
    ("q, Ctrl+C", "quitter"),
    ("Issues", ""),
    ("t", "transition"),
    ("m", "déplacer vers un sprint / le backlog"),
    ("K / J", "monter / descendre (rang)"),
    ("c / e", "créer / éditer une issue"),
    ("Sprints", ""),
    ("s", "démarrer un sprint futur"),
    ("x", "clôturer le sprint actif"),
    ("Versions (onglet 3)", ""),
    ("c / e / r", "créer / éditer / livrer la version"),
    ("Formulaires", ""),
    ("Tab, ↑/↓", "champ suivant / précédent"),
    ("←/→, Espace", "changer un choix"),
    (
        "Entrée, Ctrl+S",
        "valider (Entrée = saut de ligne dans Description)",
    ),
    ("Esc", "annuler"),
    (
        "Toute écriture",
        "est confirmée : Entrée/o/y = oui, Esc/n = non",
    ),
    ("", "(validation acceptée après un court délai d'affichage)"),
];

fn popup_block(frame: &mut Frame, area: Rect, title: &str) -> Rect {
    frame.render_widget(Clear, area);
    let block = Block::bordered()
        .title(Span::styled(format!(" {title} "), Style::new().bold()))
        .border_style(Style::new().fg(Color::Cyan));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}

fn choice_list(
    frame: &mut Frame,
    app_area: Rect,
    title: &str,
    header: Vec<Line>,
    items: Vec<String>,
    cursor: usize,
) {
    let height = (items.len() + header.len() + 3) as u16;
    let area = centered(app_area, 50, 40, height.max(6));
    let inner = popup_block(frame, area, title);
    let hlen = header.len() as u16;
    frame.render_widget(
        Paragraph::new(header),
        Rect {
            height: hlen,
            ..inner
        },
    );
    let list_area = Rect {
        y: inner.y + hlen,
        height: inner.height.saturating_sub(hlen + 1),
        ..inner
    };
    let list = List::new(items.into_iter().map(ListItem::new))
        .highlight_style(selected_style(true))
        .highlight_symbol("› ");
    frame.render_stateful_widget(
        list,
        list_area,
        &mut ListState::default().with_selected(Some(cursor)),
    );
    let foot = Rect {
        y: inner.y + inner.height.saturating_sub(1),
        height: 1,
        ..inner
    };
    frame.render_widget(
        Paragraph::new("Entrée choisir · Esc annuler".dark_gray()),
        foot,
    );
}

pub fn draw(frame: &mut Frame, app: &App) {
    let Some(popup) = &app.popup else { return };
    let full = frame.area();
    match popup {
        Popup::Help => {
            let area = centered(full, 70, 60, HELP.len() as u16 + 2);
            let inner = popup_block(frame, area, "Aide — raccourcis");
            let lines: Vec<Line> = HELP
                .iter()
                .map(|(k, v)| {
                    if v.is_empty() {
                        Line::from(Span::styled(*k, Style::new().fg(Color::Cyan).bold()))
                    } else {
                        Line::from(vec![
                            Span::styled(format!("  {k:<20}"), Style::new().bold()),
                            Span::raw(*v),
                        ])
                    }
                })
                .collect();
            frame.render_widget(Paragraph::new(lines), inner);
        }
        Popup::Detail(issue) => {
            let area = centered(full, 75, 50, full.height.saturating_sub(4).max(10));
            let inner = popup_block(frame, area, &issue.key);
            let field = |k: &str, v: String| {
                Line::from(vec![
                    Span::styled(format!("{k:<12}"), Style::new().bold()),
                    Span::raw(v),
                ])
            };
            let sprint = match issue.sprint_id {
                Some(id) => app
                    .data
                    .sprint_name(id)
                    .map_or(format!("#{id}"), str::to_string),
                None => "Backlog".into(),
            };
            let versions = if issue.fix_versions.is_empty() {
                "—".into()
            } else {
                issue
                    .fix_versions
                    .iter()
                    .map(|v| v.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let mut lines = vec![
                Line::from(Span::styled(
                    issue.summary.clone(),
                    Style::new().add_modifier(Modifier::BOLD),
                )),
                Line::raw(""),
                field("Type", issue.issue_type.name.clone()),
                Line::from(vec![
                    Span::styled(format!("{:<12}", "Statut"), Style::new().bold()),
                    Span::styled(
                        issue.status.name.clone(),
                        Style::new().fg(category_color(issue.status.category)),
                    ),
                ]),
                field(
                    "Priorité",
                    issue.priority.clone().unwrap_or_else(|| "—".into()),
                ),
                field(
                    "Assigné",
                    issue
                        .assignee
                        .as_ref()
                        .map_or("—".into(), |u| u.display_name.clone()),
                ),
                field("Points", fmt_points(issue.story_points)),
                field("Version", versions),
                field("Sprint", sprint),
                Line::raw(""),
                Line::from(Span::styled(
                    "Description",
                    Style::new().bold().underlined(),
                )),
            ];
            lines.extend(
                issue
                    .description
                    .as_deref()
                    .unwrap_or("(aucune)")
                    .lines()
                    .map(|l| Line::raw(l.to_string())),
            );
            let body = Rect {
                height: inner.height.saturating_sub(1),
                ..inner
            };
            frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), body);
            let foot = Rect {
                y: inner.y + inner.height.saturating_sub(1),
                height: 1,
                ..inner
            };
            frame.render_widget(
                Paragraph::new("t transition · m déplacer · e éditer · Esc fermer".dark_gray()),
                foot,
            );
        }
        Popup::Transitions { key, items, cursor } => match items {
            None => {
                let area = centered(full, 40, 30, 3);
                let inner = popup_block(frame, area, &format!("Transitions de {key}"));
                frame.render_widget(Paragraph::new("Chargement…"), inner);
            }
            Some(list) => {
                let labels = list
                    .iter()
                    .map(|t| {
                        // Écran de transition (champs requis) : la TUI ne sait pas le remplir.
                        let warn = if t.has_screen {
                            "  (écran Jira requis : peut échouer)"
                        } else {
                            ""
                        };
                        format!("{}  → {}{warn}", t.name, t.to.name)
                    })
                    .collect();
                choice_list(
                    frame,
                    full,
                    &format!("Transitions de {key}"),
                    vec![],
                    labels,
                    *cursor,
                );
            }
        },
        Popup::Move {
            keys,
            dests,
            cursor,
        } => {
            let labels = dests.iter().map(|d| d.label().to_string()).collect();
            choice_list(
                frame,
                full,
                &format!("Déplacer {}", keys.join(", ")),
                vec![],
                labels,
                *cursor,
            );
        }
        Popup::PickSprint { sprints, cursor } => {
            let labels = sprints.iter().map(|s| s.name.clone()).collect();
            choice_list(frame, full, "Sprint à démarrer", vec![], labels, *cursor);
        }
        Popup::CloseSprint {
            sprint,
            unfinished,
            done,
            dests,
            cursor,
        } => {
            let header = vec![
                Line::raw(format!(
                    "{done} issue(s) terminée(s), {} non terminée(s).",
                    unfinished.len()
                )),
                Line::raw("Destination des issues non terminées :"),
            ];
            let labels = dests.iter().map(|d| d.label().to_string()).collect();
            choice_list(
                frame,
                full,
                &format!("Clôturer {}", sprint.name),
                header,
                labels,
                *cursor,
            );
        }
        Popup::ReleaseVersion {
            version,
            unfinished,
            dests,
            cursor,
        } => {
            let header = vec![
                Line::raw(format!(
                    "≈ {} issue(s) non terminée(s) d'après leur statut : {}",
                    unfinished.len(),
                    unfinished.join(", ")
                )),
                Line::raw("Jira déplacera les issues non résolues vers :"),
            ];
            let labels = dests.iter().map(|d| d.label.clone()).collect();
            choice_list(
                frame,
                full,
                &format!("Livrer {}", version.name),
                header,
                labels,
                *cursor,
            );
        }
        Popup::Form(form) => draw_form(frame, full, form),
        Popup::QuitConfirm { .. } => {
            let area = centered(full, 50, 44, 5);
            let inner = popup_block(frame, area, "Quitter ?");
            let text = vec![
                Line::raw("Le formulaire contient des modifications non enregistrées."),
                Line::raw(""),
                Line::from(vec![
                    Span::styled("Entrée/o", Style::new().fg(Color::Red).bold()),
                    Span::raw(" quitter sans enregistrer · "),
                    Span::styled("Esc/n", Style::new().fg(Color::Green).bold()),
                    Span::raw(" revenir"),
                ]),
            ];
            frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), inner);
        }
        Popup::Confirm { op, .. } => {
            let (title, lines) = op.describe();
            let area = centered(full, 50, 44, lines.len() as u16 + 5);
            let inner = popup_block(frame, area, &format!("Confirmer — {title}"));
            let mut text: Vec<Line> = lines.into_iter().map(Line::raw).collect();
            text.push(Line::raw(""));
            text.push(Line::from(vec![
                Span::styled("Entrée/o", Style::new().fg(Color::Green).bold()),
                Span::raw(" confirmer · "),
                Span::styled("Esc/n", Style::new().fg(Color::Red).bold()),
                Span::raw(" annuler"),
            ]));
            frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), inner);
        }
    }
}

fn draw_form(frame: &mut Frame, full: Rect, form: &Form) {
    const LABEL_W: u16 = 24;
    let multi_rows = 6u16;
    let height = form
        .fields
        .iter()
        .map(|f| {
            if matches!(f.kind, FieldKind::Multi(_)) {
                multi_rows + 1
            } else {
                1
            }
        })
        .sum::<u16>()
        + 5;
    let area = centered(full, 70, 60, height);
    let inner = popup_block(frame, area, &form.title);
    let mut y = inner.y;
    let value_w = inner.width.saturating_sub(LABEL_W);
    for (i, field) in form.fields.iter().enumerate() {
        let focused = i == form.focus;
        let label_style = if focused {
            Style::new().fg(Color::Cyan).bold()
        } else {
            Style::new().bold()
        };
        let row = |y: u16| Rect {
            x: inner.x,
            y,
            width: inner.width,
            height: 1,
        };
        if y >= inner.y + inner.height {
            break;
        }
        let label = Span::styled(
            format!("{:<w$}", field.label, w = LABEL_W as usize),
            label_style,
        );
        let value_style = if focused {
            Style::new().bg(Color::DarkGray)
        } else {
            Style::new()
        };
        match &field.kind {
            FieldKind::Text(t) => {
                frame.render_widget(
                    Paragraph::new(Line::from(vec![
                        label,
                        Span::styled(
                            format!("{:<w$}", t.value(), w = value_w as usize),
                            value_style,
                        ),
                    ])),
                    row(y),
                );
                if focused {
                    let col = t.cursor_line_col().1 as u16;
                    frame.set_cursor_position(Position::new(
                        inner.x + LABEL_W + col.min(value_w),
                        y,
                    ));
                }
                y += 1;
            }
            FieldKind::Choice { options, selected } => {
                let v = options
                    .get(*selected)
                    .map_or("(aucun)", |o| o.label.as_str());
                frame.render_widget(
                    Paragraph::new(Line::from(vec![
                        label,
                        Span::styled(format!("◀ {v} ▶"), value_style),
                    ])),
                    row(y),
                );
                y += 1;
            }
            FieldKind::ReadOnly(v) => {
                frame.render_widget(
                    Paragraph::new(Line::from(vec![label, Span::raw(v.clone()).dark_gray()])),
                    row(y),
                );
                y += 1;
            }
            FieldKind::Multi(t) => {
                frame.render_widget(Paragraph::new(Line::from(label)), row(y));
                y += 1;
                let rows = multi_rows.min((inner.y + inner.height).saturating_sub(y));
                let box_area = Rect {
                    x: inner.x + 2,
                    y,
                    width: inner.width.saturating_sub(2),
                    height: rows,
                };
                let (line, col) = t.cursor_line_col();
                let scroll = (line as u16).saturating_sub(rows.saturating_sub(1));
                frame.render_widget(
                    Paragraph::new(t.value().to_string())
                        .style(value_style)
                        .scroll((scroll, 0)),
                    box_area,
                );
                if focused && rows > 0 {
                    frame.set_cursor_position(Position::new(
                        box_area.x + (col as u16).min(box_area.width.saturating_sub(1)),
                        y + line as u16 - scroll,
                    ));
                }
                y += rows;
            }
        }
    }
    let foot_y = inner.y + inner.height.saturating_sub(1);
    if let Some(e) = &form.error {
        let ey = foot_y.saturating_sub(1);
        frame.render_widget(
            Paragraph::new(Span::styled(format!("⚠ {e}"), Style::new().fg(Color::Red))),
            Rect {
                y: ey,
                height: 1,
                ..inner
            },
        );
    }
    frame.render_widget(
        Paragraph::new("Tab champ suivant · Entrée/Ctrl+S valider · Esc annuler".dark_gray()),
        Rect {
            y: foot_y,
            height: 1,
            ..inner
        },
    );
}
