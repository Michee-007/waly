//! Horloge locale en français — la source de vérité temporelle de Waly
//! (le grief n°1 de l'ancien monde était l'heure fausse). Partagée : outil
//! `heure` du core, et horodatage des messages côté voix (R3).

use chrono::{DateTime, Datelike, Local, TimeZone, Timelike};

const JOURS: [&str; 7] =
    ["lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche"];
const MOIS: [&str; 12] = [
    "janvier", "fevrier", "mars", "avril", "mai", "juin", "juillet", "aout", "septembre",
    "octobre", "novembre", "decembre",
];

/// « dimanche 5 juillet 2026, 01:03 »
pub fn french_timestamp() -> String {
    format_french(&Local::now())
}

pub fn format_french<Tz: TimeZone>(t: &DateTime<Tz>) -> String {
    format!(
        "{} {} {} {}, {:02}:{:02}",
        JOURS[t.weekday().num_days_from_monday() as usize],
        t.day(),
        MOIS[t.month0() as usize],
        t.year(),
        t.hour(),
        t.minute()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_francais() {
        let t = chrono::NaiveDate::from_ymd_opt(2026, 7, 5)
            .unwrap()
            .and_hms_opt(1, 3, 0)
            .unwrap()
            .and_utc();
        assert_eq!(format_french(&t), "dimanche 5 juillet 2026, 01:03");
    }
}
