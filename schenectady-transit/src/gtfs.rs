use anyhow::{Context, Result};
use chrono::{Datelike, NaiveDate};
use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::util::parse_gtfs_time;

#[derive(Debug, Clone)]
pub struct Stop {
    #[allow(dead_code)]
    pub id: String,
    pub name: String,
    pub lat: f64,
    pub lon: f64,
}

#[derive(Debug, Clone)]
pub struct Route {
    #[allow(dead_code)]
    pub id: String,
    pub short_name: String,
    pub long_name: String,
}

#[derive(Debug, Clone)]
pub struct StopTime {
    pub stop_id: String,
    pub stop_sequence: u32,
    pub arrival_secs: u32,
    pub departure_secs: u32,
}

pub struct GtfsData {
    pub stops: HashMap<String, Stop>,
    pub routes: HashMap<String, Route>,
    pub trip_route: HashMap<String, String>,
    pub trip_stops: HashMap<String, Vec<StopTime>>,
    pub stop_departures: HashMap<String, Vec<(u32, String)>>,
}

impl GtfsData {
    pub fn load(dir: &str, date: NaiveDate) -> Result<Self> {
        let dir = Path::new(dir);
        let stops = load_stops(dir)?;
        let routes = load_routes(dir)?;
        let (trip_route, active_trips) = load_trips(dir, date)?;
        let (trip_stops, stop_departures) = load_stop_times(dir, &active_trips)?;
        Ok(Self { stops, routes, trip_route, trip_stops, stop_departures })
    }

    pub fn active_trip_count(&self) -> usize {
        self.trip_stops.len()
    }

    pub fn route_name(&self, trip_id: &str) -> &str {
        self.trip_route
            .get(trip_id)
            .and_then(|rid| self.routes.get(rid))
            .map(|r| if !r.short_name.is_empty() { r.short_name.as_str() } else { r.long_name.as_str() })
            .unwrap_or("?")
    }
}

fn load_stops(dir: &Path) -> Result<HashMap<String, Stop>> {
    let mut rdr = csv::Reader::from_path(dir.join("stops.txt")).context("stops.txt")?;
    let headers = rdr.headers()?.clone();
    let mut stops = HashMap::new();
    for rec in rdr.records() {
        let rec = rec?;
        let id = col(&rec, &headers, "stop_id")?;
        let name = col(&rec, &headers, "stop_name").unwrap_or_default();
        let lat: f64 = col(&rec, &headers, "stop_lat")?.parse().context("stop_lat")?;
        let lon: f64 = col(&rec, &headers, "stop_lon")?.parse().context("stop_lon")?;
        stops.insert(id.clone(), Stop { id, name, lat, lon });
    }
    Ok(stops)
}

fn load_routes(dir: &Path) -> Result<HashMap<String, Route>> {
    let mut rdr = csv::Reader::from_path(dir.join("routes.txt")).context("routes.txt")?;
    let headers = rdr.headers()?.clone();
    let mut routes = HashMap::new();
    for rec in rdr.records() {
        let rec = rec?;
        let id = col(&rec, &headers, "route_id")?;
        let short_name = col(&rec, &headers, "route_short_name").unwrap_or_default();
        let long_name = col(&rec, &headers, "route_long_name").unwrap_or_default();
        routes.insert(id.clone(), Route { id, short_name, long_name });
    }
    Ok(routes)
}

fn load_trips(dir: &Path, date: NaiveDate) -> Result<(HashMap<String, String>, HashSet<String>)> {
    let active_services = active_services(dir, date)?;
    let mut rdr = csv::Reader::from_path(dir.join("trips.txt")).context("trips.txt")?;
    let headers = rdr.headers()?.clone();
    let mut trip_route: HashMap<String, String> = HashMap::new();
    let mut active_trips: HashSet<String> = HashSet::new();
    for rec in rdr.records() {
        let rec = rec?;
        let trip_id = col(&rec, &headers, "trip_id")?;
        let route_id = col(&rec, &headers, "route_id")?;
        let service_id = col(&rec, &headers, "service_id")?;
        trip_route.insert(trip_id.clone(), route_id);
        if active_services.contains(&service_id) {
            active_trips.insert(trip_id);
        }
    }
    Ok((trip_route, active_trips))
}

fn active_services(dir: &Path, date: NaiveDate) -> Result<HashSet<String>> {
    let weekday_col = match date.weekday() {
        chrono::Weekday::Mon => "monday",
        chrono::Weekday::Tue => "tuesday",
        chrono::Weekday::Wed => "wednesday",
        chrono::Weekday::Thu => "thursday",
        chrono::Weekday::Fri => "friday",
        chrono::Weekday::Sat => "saturday",
        chrono::Weekday::Sun => "sunday",
    };
    let date_int: u32 = date.format("%Y%m%d").to_string().parse()?;

    let mut active: HashSet<String> = HashSet::new();
    let mut removed: HashSet<String> = HashSet::new();

    let cal_path = dir.join("calendar.txt");
    if cal_path.exists() {
        let mut rdr = csv::Reader::from_path(&cal_path).context("calendar.txt")?;
        let headers = rdr.headers()?.clone();
        for rec in rdr.records() {
            let rec = rec?;
            let service_id = col(&rec, &headers, "service_id")?;
            let start: u32 = col(&rec, &headers, "start_date")?.parse()?;
            let end: u32 = col(&rec, &headers, "end_date")?.parse()?;
            let runs = col(&rec, &headers, weekday_col).unwrap_or_default();
            if date_int >= start && date_int <= end && runs.trim() == "1" {
                active.insert(service_id);
            }
        }
    }

    let cal_dates_path = dir.join("calendar_dates.txt");
    if cal_dates_path.exists() {
        let mut rdr = csv::Reader::from_path(&cal_dates_path).context("calendar_dates.txt")?;
        let headers = rdr.headers()?.clone();
        for rec in rdr.records() {
            let rec = rec?;
            let service_id = col(&rec, &headers, "service_id")?;
            let rec_date: u32 = col(&rec, &headers, "date")?.parse()?;
            let exception_type: u8 = col(&rec, &headers, "exception_type")?.parse()?;
            if rec_date == date_int {
                match exception_type {
                    1 => { active.insert(service_id); }
                    2 => { removed.insert(service_id); }
                    _ => {}
                }
            }
        }
    }

    for sid in &removed {
        active.remove(sid);
    }
    Ok(active)
}

fn load_stop_times(
    dir: &Path,
    active_trips: &HashSet<String>,
) -> Result<(HashMap<String, Vec<StopTime>>, HashMap<String, Vec<(u32, String)>>)> {
    let mut trip_stops: HashMap<String, Vec<StopTime>> = HashMap::new();
    let mut rdr = csv::Reader::from_path(dir.join("stop_times.txt")).context("stop_times.txt")?;
    let headers = rdr.headers()?.clone();

    for rec in rdr.records() {
        let rec = rec?;
        let trip_id = col(&rec, &headers, "trip_id")?;
        if !active_trips.contains(&trip_id) {
            continue;
        }
        let stop_id = col(&rec, &headers, "stop_id")?;
        let seq: u32 = col(&rec, &headers, "stop_sequence")?.parse().context("stop_sequence")?;
        let arr = col(&rec, &headers, "arrival_time").unwrap_or_default();
        let dep = col(&rec, &headers, "departure_time").unwrap_or_default();
        let arrival_secs = parse_gtfs_time(&arr).unwrap_or(0);
        let departure_secs = parse_gtfs_time(&dep).unwrap_or(arrival_secs);

        trip_stops.entry(trip_id).or_default()
            .push(StopTime { stop_id, stop_sequence: seq, arrival_secs, departure_secs });
    }

    for stops in trip_stops.values_mut() {
        stops.sort_by_key(|s| s.stop_sequence);
    }

    let mut stop_departures: HashMap<String, Vec<(u32, String)>> = HashMap::new();
    for (trip_id, stops) in &trip_stops {
        for st in stops {
            stop_departures.entry(st.stop_id.clone()).or_default()
                .push((st.departure_secs, trip_id.clone()));
        }
    }
    for deps in stop_departures.values_mut() {
        deps.sort_by_key(|(t, _)| *t);
    }

    Ok((trip_stops, stop_departures))
}

fn col(rec: &csv::StringRecord, headers: &csv::StringRecord, name: &str) -> Result<String> {
    let idx = headers.iter().position(|h| h.trim() == name)
        .with_context(|| format!("column '{name}' not found"))?;
    Ok(rec.get(idx).unwrap_or("").trim().to_string())
}
