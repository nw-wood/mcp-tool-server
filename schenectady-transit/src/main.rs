mod geocoder;
mod gtfs;
mod router;
mod util;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::Path;

const GTFS_DIR: &str = "data/gtfs";
const GEOCODER_DB: &str = "data/geocoder.sqlite";
const GTFS_URL: &str = "https://www.cdta.org/schedules/google_transit.zip";
const MAX_WALK_MILES: f64 = 0.25;

#[derive(Parser)]
#[command(name = "schenectady-transit", about = "Offline CDTA bus route planner")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Find bus routes between two addresses
    Route {
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
        /// Arrive by this time (HH:MM) or "now" to depart immediately
        #[arg(long, default_value = "now")]
        arrive_by: String,
        /// Maximum total travel time in minutes (walking + waiting + riding)
        #[arg(long, default_value = "90")]
        max_minutes: u32,
        /// Maximum number of bus transfers
        #[arg(long, default_value = "2")]
        max_transfers: u32,
    },
    /// Download and cache the latest CDTA GTFS feed
    UpdateGtfs,
    /// Build the local address geocoder index from the OSM PBF extract
    BuildGeocoder,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Route { from, to, arrive_by, max_minutes, max_transfers } => {
            cmd_route(&from, &to, &arrive_by, max_minutes, max_transfers)
        }
        Command::UpdateGtfs => cmd_update_gtfs(),
        Command::BuildGeocoder => cmd_build_geocoder(),
    }
}

fn cmd_route(from: &str, to: &str, arrive_by: &str, max_minutes: u32, max_transfers: u32) -> Result<()> {
    if !Path::new(GTFS_DIR).exists() {
        anyhow::bail!("GTFS data not found. Run 'schenectady-transit update-gtfs' first.");
    }
    if !Path::new(GEOCODER_DB).exists() {
        anyhow::bail!("Geocoder index not found. Run 'schenectady-transit build-geocoder' first.");
    }

    let ts_path = Path::new(GTFS_DIR).join("downloaded_at.txt");
    if let Ok(ts) = std::fs::read_to_string(&ts_path) {
        println!("GTFS data last updated: {}", ts.trim());
    }

    let gc = geocoder::Geocoder::open(GEOCODER_DB)?;

    let origin = gc.geocode(from)
        .with_context(|| format!("Could not geocode origin: {from}"))?;
    let destination = gc.geocode(to)
        .with_context(|| format!("Could not geocode destination: {to}"))?;

    println!("From: {from} ({:.5}, {:.5})", origin.lat, origin.lon);
    println!("To:   {to} ({:.5}, {:.5})", destination.lat, destination.lon);

    let now = chrono::Local::now();
    let now_secs = util::secs_since_midnight(&now);

    let arrive_by_secs: u32 = if arrive_by == "now" {
        now_secs + max_minutes * 60
    } else {
        util::parse_hhmm(arrive_by)
            .with_context(|| format!("Invalid arrive_by time '{arrive_by}'. Use HH:MM or 'now'."))?
    };

    let query_date = now.date_naive();

    print!("Loading transit data... ");
    let gtfs = gtfs::GtfsData::load(GTFS_DIR, query_date)?;
    println!("{} stops, {} active trips.", gtfs.stops.len(), gtfs.active_trip_count());

    let results = router::find_routes(
        &gtfs,
        origin,
        destination,
        now_secs,
        arrive_by_secs,
        max_minutes,
        max_transfers,
        MAX_WALK_MILES,
    )?;

    if results.is_empty() {
        println!(
            "\nNo routes found arriving by {} within {} minutes.",
            util::fmt_time(arrive_by_secs),
            max_minutes
        );
        return Ok(());
    }

    println!("\nFound {} option(s):\n", results.len());
    for (i, itin) in results.iter().enumerate() {
        print_itinerary(i + 1, itin);
    }

    Ok(())
}

fn print_itinerary(n: usize, itin: &router::Itinerary) {
    println!(
        "Option {} — Depart {} · Arrive {} · {} min total",
        n,
        util::fmt_time(itin.depart_secs),
        util::fmt_time(itin.arrive_secs),
        (itin.arrive_secs - itin.depart_secs) / 60,
    );
    for leg in &itin.legs {
        match leg {
            router::Leg::Walk(w) if w.from_name == w.to_name => continue,
            router::Leg::Walk(w) => println!(
                "  Walk  {:.2} mi (~{} min)  {} → {}",
                w.distance_miles,
                w.duration_secs / 60,
                w.from_name,
                w.to_name,
            ),
            router::Leg::Bus(b) => println!(
                "  Bus   Route {} · Depart {} · {} stop(s) · Arrive {} at {}",
                b.route_name,
                util::fmt_time(b.depart_secs),
                b.stop_count,
                util::fmt_time(b.arrive_secs),
                b.to_stop,
            ),
        }
    }
    println!();
}

fn cmd_update_gtfs() -> Result<()> {
    use std::io::Read;

    println!("Downloading GTFS feed from CDTA...");
    let resp = reqwest::blocking::get(GTFS_URL)
        .context("Failed to download GTFS feed")?;
    if !resp.status().is_success() {
        anyhow::bail!("HTTP {}: {}", resp.status(), GTFS_URL);
    }
    let bytes = resp.bytes().context("Failed to read response body")?;
    println!("Downloaded {} KB. Extracting...", bytes.len() / 1024);

    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor).context("Failed to open GTFS zip")?;

    std::fs::create_dir_all(GTFS_DIR)?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let outpath = Path::new(GTFS_DIR).join(file.name());
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;
        std::fs::write(&outpath, &buf)?;
    }

    let ts = chrono::Local::now().to_rfc3339();
    std::fs::write(Path::new(GTFS_DIR).join("downloaded_at.txt"), &ts)?;
    println!("GTFS feed extracted to {GTFS_DIR}/");
    println!("Timestamp: {ts}");
    Ok(())
}

fn cmd_build_geocoder() -> Result<()> {
    let pbf_path = find_pbf_file()?;
    println!("Building geocoder index from {}...", pbf_path.display());
    std::fs::create_dir_all("data")?;
    geocoder::build_index(&pbf_path, GEOCODER_DB)?;
    println!("Geocoder index written to {GEOCODER_DB}");
    Ok(())
}

fn find_pbf_file() -> Result<std::path::PathBuf> {
    for entry in std::fs::read_dir("data")
        .context("Could not read data/ directory. Are you in the schenectady-transit directory?")?
    {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("pbf") {
            return Ok(path);
        }
    }
    anyhow::bail!("No .pbf file found in data/. Download one from BBBike and place it in schenectady-transit/data/");
}
