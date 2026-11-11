//! Petits utilitaires de dates sans dépendance (HTTP-date pour `Retry-After`, ISO 8601 pour le mock).

/// Nombre de jours depuis 1970-01-01 (algorithme de H. Hinnant, calendrier grégorien proleptique).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = i64::from(m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        _ => 0,
    }
}

fn num<T: std::str::FromStr>(s: &str) -> Option<T> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

fn epoch_secs(y: i64, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> Option<i64> {
    if !(1..=12).contains(&mo) || d == 0 || d > days_in_month(y, mo) || h > 23 || mi > 59 || s > 60
    {
        return None;
    }
    Some(days_from_civil(y, mo, d) * 86_400 + i64::from(h * 3600 + mi * 60 + s))
}

/// HTTP-date au format IMF-fixdate (`Sun, 06 Nov 1994 08:49:37 GMT`) -> secondes Unix.
pub(crate) fn parse_http_date(s: &str) -> Option<i64> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    let [_, day, month, year, time, "GMT"] = parts.as_slice() else {
        return None;
    };
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let mo = MONTHS.iter().position(|m| m.eq_ignore_ascii_case(month))? as u32 + 1;
    let hms: Vec<&str> = time.split(':').collect();
    let [h, mi, sec] = hms.as_slice() else {
        return None;
    };
    epoch_secs(num(year)?, mo, num(day)?, num(h)?, num(mi)?, num(sec)?)
}

/// ISO 8601 complet (`2026-11-16T08:00:00.000Z`, `2026-11-16T09:00:00+01:00`) -> millisecondes Unix.
pub(crate) fn parse_iso8601_millis(s: &str) -> Option<i64> {
    let (date, rest) = s.split_once('T')?;
    let dp: Vec<&str> = date.split('-').collect();
    let [y, mo, d] = dp.as_slice() else {
        return None;
    };
    if y.len() != 4 || mo.len() != 2 || d.len() != 2 {
        return None;
    }
    // Séparer le fuseau : `Z` ou `±HH:MM` / `±HHMM`.
    let (time, offset_secs) = if let Some(t) = rest.strip_suffix('Z') {
        (t, 0i64)
    } else {
        let idx = rest.rfind(['+', '-'])?;
        let (t, off) = rest.split_at(idx);
        let sign = if off.starts_with('-') { -1 } else { 1 };
        let off = off[1..].replace(':', "");
        if off.len() != 4 {
            return None;
        }
        let oh: i64 = num(&off[..2])?;
        let om: i64 = num(&off[2..])?;
        if oh > 23 || om > 59 {
            return None;
        }
        (t, sign * (oh * 3600 + om * 60))
    };
    let (hms, frac) = match time.split_once('.') {
        Some((a, b)) => (a, Some(b)),
        None => (time, None),
    };
    let tp: Vec<&str> = hms.split(':').collect();
    let (h, mi, sec) = match tp.as_slice() {
        [h, mi, sec] => (*h, *mi, *sec),
        [h, mi] => (*h, *mi, "00"),
        _ => return None,
    };
    if h.len() != 2 || mi.len() != 2 || sec.len() != 2 {
        return None;
    }
    let millis = match frac {
        None => 0,
        Some(f) => {
            if f.is_empty() || f.len() > 9 {
                return None;
            }
            let padded = format!("{f:0<3}");
            num::<i64>(&padded[..3])?
        }
    };
    let secs = epoch_secs(num(y)?, num(mo)?, num(d)?, num(h)?, num(mi)?, num(sec)?)?;
    Some((secs - offset_secs) * 1000 + millis)
}

/// Même instant (comparaison en UTC) ; à défaut de pouvoir parser, comparaison exacte des chaînes.
fn same_instant(a: &str, b: &str) -> bool {
    match (parse_iso8601_millis(a), parse_iso8601_millis(b)) {
        (Some(x), Some(y)) => x == y,
        _ => a.trim() == b.trim(),
    }
}

/// Jira ignore sans erreur (200) les dates envoyées pour un sprint clôturé. Si le sprint renvoyé est
/// clôturé et qu'une date demandée diffère de celle de la réponse, renvoie une erreur explicite
/// (le nom et l'objectif, eux, ont été appliqués).
pub(crate) fn check_closed_sprint_dates(
    update: &crate::model::SprintUpdate,
    sprint: &crate::model::Sprint,
) -> crate::Result<()> {
    if sprint.state != crate::model::SprintState::Closed {
        return Ok(());
    }
    let ignored = |requested: &Option<String>, actual: &Option<String>| match (requested, actual) {
        (None, _) => false,
        (Some(r), Some(a)) => !same_instant(r, a),
        (Some(_), None) => true,
    };
    if ignored(&update.start_date, &sprint.start_date)
        || ignored(&update.end_date, &sprint.end_date)
    {
        return Err(crate::Error::Invalid(format!(
            "dates ignorées par Jira : sprint fermé (sprint {} ; nom et objectif éventuels appliqués)",
            sprint.id
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_date() {
        assert_eq!(
            parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT"),
            Some(784_111_777)
        );
        assert_eq!(parse_http_date("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
        assert_eq!(parse_http_date("Sun, 06 Nov 1994 08:49:37 CET"), None);
        assert_eq!(parse_http_date("7"), None);
        assert_eq!(parse_http_date("Sun, 31 Feb 1994 08:49:37 GMT"), None);
    }

    #[test]
    fn iso8601() {
        assert_eq!(parse_iso8601_millis("1970-01-01T00:00:00.000Z"), Some(0));
        assert_eq!(parse_iso8601_millis("1970-01-01T00:00:01.5Z"), Some(1500));
        assert_eq!(parse_iso8601_millis("1970-01-01T01:00:00+01:00"), Some(0));
        assert_eq!(
            parse_iso8601_millis("1970-01-01T00:00:00-0030"),
            Some(1_800_000)
        );
        assert!(parse_iso8601_millis("2026-11-16T08:00:00.000Z").is_some());
        assert_eq!(parse_iso8601_millis("2026-11-16"), None);
        assert_eq!(parse_iso8601_millis("2026-13-16T08:00:00Z"), None);
        assert_eq!(parse_iso8601_millis("demain"), None);
        assert_eq!(parse_iso8601_millis("2026-11-16T08:00:00"), None);
    }
}
