use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::Path;

#[derive(Debug, Clone, Copy)]
pub struct Coord {
    pub lat: f64,
    pub lon: f64,
}

pub struct Geocoder {
    conn: Connection,
}

impl Geocoder {
    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open(path).context("opening geocoder.sqlite")?;
        Ok(Self { conn })
    }

    /// Geocode a free-form address string to coordinates.
    /// Tries housenumber + street first, falls back to street centroid.
    pub fn geocode(&self, address: &str) -> Result<Coord> {
        let (housenumber, street) = parse_address(address);
        let street_norm = normalize_street(&street);

        if let Some(n) = housenumber {
            if let Some(coord) = self.query_exact(&n, &street_norm)? {
                return Ok(coord);
            }
        }

        self.query_street_centroid(&street_norm)?
            .with_context(|| format!("Address not found: '{address}'"))
    }

    fn query_exact(&self, housenumber: &str, street_norm: &str) -> Result<Option<Coord>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT lat, lon FROM addresses WHERE street_norm = ?1 AND housenumber = ?2 LIMIT 1",
        )?;
        let mut rows = stmt.query(params![street_norm, housenumber])?;
        if let Some(row) = rows.next()? {
            return Ok(Some(Coord { lat: row.get(0)?, lon: row.get(1)? }));
        }
        Ok(None)
    }

    fn query_street_centroid(&self, street_norm: &str) -> Result<Option<Coord>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT AVG(lat), AVG(lon) FROM addresses WHERE street_norm = ?1",
        )?;
        let mut rows = stmt.query(params![street_norm])?;
        if let Some(row) = rows.next()? {
            let lat: Option<f64> = row.get(0)?;
            let lon: Option<f64> = row.get(1)?;
            if let (Some(lat), Some(lon)) = (lat, lon) {
                return Ok(Some(Coord { lat, lon }));
            }
        }
        Ok(None)
    }
}

/// Build geocoder SQLite index from a PBF file.
pub fn build_index(pbf_path: &Path, db_path: &str) -> Result<()> {
    use osmpbf::{Element, ElementReader};

    let conn = Connection::open(db_path).context("creating geocoder.sqlite")?;
    conn.execute_batch(
        "DROP TABLE IF EXISTS addresses;
         CREATE TABLE addresses (
             housenumber TEXT,
             street      TEXT,
             street_norm TEXT NOT NULL,
             city        TEXT,
             lat         REAL NOT NULL,
             lon         REAL NOT NULL
         );",
    )?;

    let reader = ElementReader::from_path(pbf_path).context("opening PBF file")?;

    let mut count = 0u64;
    conn.execute("BEGIN", [])?;

    {
        let conn_ref = &conn;
        reader.for_each(|element| {
            let info: Option<(Option<String>, Option<String>, Option<String>, f64, f64)> =
                match &element {
                    Element::Node(n) => {
                        let mut hn = None;
                        let mut st = None;
                        let mut cy = None;
                        for (k, v) in n.tags() {
                            match k {
                                "addr:housenumber" => hn = Some(v.to_string()),
                                "addr:street" => st = Some(v.to_string()),
                                "addr:city" => cy = Some(v.to_string()),
                                _ => {}
                            }
                        }
                        Some((hn, st, cy, n.lat(), n.lon()))
                    }
                    Element::DenseNode(n) => {
                        let mut hn = None;
                        let mut st = None;
                        let mut cy = None;
                        for (k, v) in n.tags() {
                            match k {
                                "addr:housenumber" => hn = Some(v.to_string()),
                                "addr:street" => st = Some(v.to_string()),
                                "addr:city" => cy = Some(v.to_string()),
                                _ => {}
                            }
                        }
                        Some((hn, st, cy, n.lat(), n.lon()))
                    }
                    Element::Way(_) | Element::Relation(_) => None,
                };

            let (hn, st, cy, lat, lon) = match info {
                Some(x) => x,
                None => return,
            };
            let street = match st {
                Some(s) => s,
                None => return,
            };
            let street_norm = normalize_street(&street);

            conn_ref
                .execute(
                    "INSERT INTO addresses (housenumber, street, street_norm, city, lat, lon)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![hn, street, street_norm, cy, lat, lon],
                )
                .ok();

            count += 1;
            if count % 100_000 == 0 {
                eprintln!("  {count} addresses indexed...");
            }
        })?;
    }

    conn.execute("COMMIT", [])?;
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_street_norm  ON addresses(street_norm);
         CREATE INDEX IF NOT EXISTS idx_street_house ON addresses(street_norm, housenumber);",
    )?;

    println!("Indexed {count} address nodes.");
    Ok(())
}

/// Split "123 Main St, Schenectady NY" into (Some("123"), "Main St").
/// Strips city/state after comma and any trailing zip codes.
fn parse_address(address: &str) -> (Option<String>, String) {
    // Drop everything after first comma (city, state, zip)
    let base = address.split(',').next().unwrap_or(address).trim();

    // Tokenize and strip trailing zip codes (5-digit numbers).
    // State abbreviations are already gone since we split on the comma above.
    let tokens: Vec<&str> = base
        .split_whitespace()
        .filter(|t| {
            let is_zip = t.len() == 5 && t.chars().all(|c| c.is_ascii_digit());
            !is_zip
        })
        .collect();

    if tokens.is_empty() {
        return (None, base.to_string());
    }

    let first = tokens[0];
    if first.chars().all(|c| c.is_ascii_digit()) && tokens.len() > 1 {
        let street = tokens[1..].join(" ");
        (Some(first.to_string()), street)
    } else {
        (None, tokens.join(" "))
    }
}

/// Normalize a street name for fuzzy matching: lowercase, expand abbreviations word by word.
pub fn normalize_street(s: &str) -> String {
    s.to_lowercase()
        .replace('.', "")
        .split_whitespace()
        .map(|w| match w {
            "st"                => "street",
            "ave" | "av"        => "avenue",
            "blvd"              => "boulevard",
            "rd"                => "road",
            "dr"                => "drive",
            "ln"                => "lane",
            "ct"                => "court",
            "pl"                => "place",
            "pkwy"              => "parkway",
            "hwy"               => "highway",
            "expy"              => "expressway",
            "tpke" | "tpk"      => "turnpike",
            "n"                 => "north",
            "s"                 => "south",
            "e"                 => "east",
            "w"                 => "west",
            other               => other,
        })
        .collect::<Vec<_>>()
        .join(" ")
}
