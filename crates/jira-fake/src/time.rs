//! Dates minimales (UTC) sans dépendance externe.

use std::time::{SystemTime, UNIX_EPOCH};

/// Jours depuis 1970-01-01 -> (année, mois, jour). Algorithme de H. Hinnant.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Date du jour, `YYYY-MM-DD`.
pub fn today() -> String {
    let (y, m, d) = civil_from_days(now_millis().div_euclid(86_400_000));
    format!("{y:04}-{m:02}-{d:02}")
}

/// Horodatage courant au format Jira agile (`2026-11-16T16:58:03.120Z`).
pub fn now_iso() -> String {
    let ms = now_millis();
    let (y, mo, d) = civil_from_days(ms.div_euclid(86_400_000));
    let rem = ms.rem_euclid(86_400_000);
    let (h, mi, s, milli) = (
        rem / 3_600_000,
        rem / 60_000 % 60,
        rem / 1000 % 60,
        rem % 1000,
    );
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{milli:03}Z")
}

/// `2026-12-01` -> `01/Dec/26` (format d'affichage `userReleaseDate`).
pub fn user_date(iso: &str) -> String {
    const M: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let mut it = iso.splitn(3, '-');
    match (it.next(), it.next(), it.next()) {
        (Some(y), Some(m), Some(d)) if y.len() == 4 => {
            let month = m
                .parse::<usize>()
                .ok()
                .and_then(|m| M.get(m.wrapping_sub(1)));
            match month {
                Some(mon) => format!("{d}/{mon}/{}", &y[2..]),
                None => iso.to_owned(),
            }
        }
        _ => iso.to_owned(),
    }
}

/// Validation légère `YYYY-MM-DD`.
pub fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_768), (2026, 11, 11));
        assert_eq!(user_date("2026-12-01"), "01/Dec/26");
        assert!(is_date("2027-02-28"));
        assert!(!is_date("2027-2-28"));
        assert_eq!(now_iso().len(), 24);
    }
}
