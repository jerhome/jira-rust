//! Dates : saisie `AAAA-MM-JJ`, conversion ISO 8601 pour l'API Agile (sprints), affichage.
//!
//! Crate `jiff` : calendrier civil + fuseau horaire système, sans dépendance C.

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Span, Zoned};

pub fn today() -> Date {
    Zoned::now().date()
}

pub fn parse_date(s: &str) -> Result<Date, String> {
    let s = s.trim();
    s.parse::<Date>()
        .map_err(|_| format!("date invalide « {s} » (format AAAA-MM-JJ)"))
}

pub fn plus_days(d: Date, days: i64) -> Date {
    d.checked_add(Span::new().days(days)).unwrap_or(d)
}

/// Date + heure locale (fuseau `tz`) au format attendu par Jira Agile, ex. `2026-11-11T09:00:00.000+01:00`.
pub fn sprint_datetime(d: Date, hour: i8, tz: &TimeZone) -> String {
    match d.at(hour, 0, 0, 0).to_zoned(tz.clone()) {
        Ok(z) => z.strftime("%Y-%m-%dT%H:%M:%S%.3f%:z").to_string(),
        Err(_) => format!("{d}T{hour:02}:00:00.000Z"),
    }
}

/// Partie date d'une chaîne ISO (`2026-11-04T09:00:00.000Z` ou `2026-11-04`).
pub fn date_part(iso: &str) -> Option<Date> {
    iso.get(..10)?.parse().ok()
}

/// Affichage court `JJ/MM/AAAA` ; renvoie la chaîne telle quelle si elle n'est pas une date.
pub fn short(iso: &str) -> String {
    match date_part(iso) {
        Some(d) => d.strftime("%d/%m/%Y").to_string(),
        None => iso.to_string(),
    }
}

/// Nombre de jours de `today` jusqu'à la date ISO (négatif si dépassée).
pub fn days_until(today: Date, iso: &str) -> Option<i64> {
    let end = date_part(iso)?;
    today.until(end).ok().map(|span| i64::from(span.get_days()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    fn parse_and_format() {
        assert_eq!(parse_date(" 2026-11-11 ").unwrap(), date(2026, 11, 11));
        assert!(parse_date("11/11/2026").unwrap_err().contains("AAAA-MM-JJ"));
        assert_eq!(plus_days(date(2026, 11, 25), 14), date(2026, 12, 9));
        assert_eq!(
            sprint_datetime(date(2026, 11, 11), 9, &TimeZone::UTC),
            "2026-11-11T09:00:00.000+00:00"
        );
        assert_eq!(short("2026-11-04T09:00:00.000Z"), "04/11/2026");
        assert_eq!(short("n/a"), "n/a");
        assert_eq!(
            days_until(date(2026, 11, 11), "2026-11-18T17:00:00.000Z"),
            Some(7)
        );
        assert_eq!(days_until(date(2026, 11, 20), "2026-11-18"), Some(-2));
    }
}
