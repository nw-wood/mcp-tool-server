use anyhow::Result;

/// Seconds since midnight for a local datetime.
pub fn secs_since_midnight(dt: &chrono::DateTime<chrono::Local>) -> u32 {
    use chrono::Timelike;
    dt.hour() * 3600 + dt.minute() * 60 + dt.second()
}

/// Parse "HH:MM" into seconds since midnight.
pub fn parse_hhmm(s: &str) -> Result<u32> {
    let parts: Vec<&str> = s.splitn(2, ':').collect();
    if parts.len() != 2 {
        anyhow::bail!("expected HH:MM, got '{s}'");
    }
    let h: u32 = parts[0].parse().map_err(|_| anyhow::anyhow!("invalid hour in '{s}'"))?;
    let m: u32 = parts[1].parse().map_err(|_| anyhow::anyhow!("invalid minute in '{s}'"))?;
    if m >= 60 {
        anyhow::bail!("minutes out of range in '{s}'");
    }
    Ok(h * 3600 + m * 60)
}

/// Parse GTFS time string "HH:MM:SS" (may exceed 24:00:00) into seconds since midnight.
pub fn parse_gtfs_time(s: &str) -> Option<u32> {
    let parts: Vec<&str> = s.splitn(3, ':').collect();
    if parts.len() != 3 {
        return None;
    }
    let h: u32 = parts[0].parse().ok()?;
    let m: u32 = parts[1].parse().ok()?;
    let sec: u32 = parts[2].parse().ok()?;
    Some(h * 3600 + m * 60 + sec)
}

/// Format seconds since midnight as "HH:MM".
pub fn fmt_time(secs: u32) -> String {
    let h = (secs / 3600) % 24;
    let m = (secs % 3600) / 60;
    format!("{h:02}:{m:02}")
}

/// Haversine distance in miles between two lat/lon points.
pub fn distance_miles(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const EARTH_RADIUS_MILES: f64 = 3958.8;
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (dlon / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().asin();
    EARTH_RADIUS_MILES * c
}

/// Walking time in seconds at 3 mph.
pub fn walk_secs(miles: f64) -> u32 {
    (miles / 3.0 * 3600.0) as u32
}
