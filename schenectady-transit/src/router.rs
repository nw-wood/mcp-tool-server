use anyhow::Result;
use std::collections::{BinaryHeap, HashMap};

use crate::geocoder::Coord;
use crate::gtfs::GtfsData;
use crate::util::{distance_miles, walk_secs};

const TRANSFER_WALK_MILES: f64 = 0.25;
const MIN_TRANSFER_WALK_MILES: f64 = 0.05;
const MAX_RESULTS: usize = 3;
/// Max trips to try from each stop to avoid exponential blow-up.
const MAX_TRIPS_PER_STOP: usize = 3;

// ─── Public types ─────────────────────────────────────────────────────────────

pub struct Itinerary {
    pub legs: Vec<Leg>,
    pub depart_secs: u32,
    pub arrive_secs: u32,
}

pub enum Leg {
    Walk(WalkLeg),
    Bus(BusLeg),
}

pub struct WalkLeg {
    pub from_name: String,
    pub to_name: String,
    pub distance_miles: f64,
    pub duration_secs: u32,
}

pub struct BusLeg {
    pub route_name: String,
    pub from_stop: String,
    pub to_stop: String,
    pub depart_secs: u32,
    pub arrive_secs: u32,
    pub stop_count: u32,
}

// ─── Internal search state ────────────────────────────────────────────────────

#[derive(Clone)]
struct State {
    arrive_secs: u32,
    stop_id: String,
    buses_taken: u32,
    legs: Vec<Leg>,
    depart_secs: u32,
}

impl PartialEq for State {
    fn eq(&self, other: &Self) -> bool {
        self.arrive_secs == other.arrive_secs
    }
}
impl Eq for State {}
impl PartialOrd for State {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for State {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Min-heap: lower arrive_secs = higher priority
        other.arrive_secs.cmp(&self.arrive_secs)
    }
}

impl Clone for Leg {
    fn clone(&self) -> Self {
        match self {
            Leg::Walk(w) => Leg::Walk(WalkLeg {
                from_name: w.from_name.clone(),
                to_name: w.to_name.clone(),
                distance_miles: w.distance_miles,
                duration_secs: w.duration_secs,
            }),
            Leg::Bus(b) => Leg::Bus(BusLeg {
                route_name: b.route_name.clone(),
                from_stop: b.from_stop.clone(),
                to_stop: b.to_stop.clone(),
                depart_secs: b.depart_secs,
                arrive_secs: b.arrive_secs,
                stop_count: b.stop_count,
            }),
        }
    }
}

// ─── Main entry point ─────────────────────────────────────────────────────────

pub fn find_routes(
    gtfs: &GtfsData,
    origin: Coord,
    destination: Coord,
    now_secs: u32,
    arrive_by_secs: u32,
    max_minutes: u32,
    max_transfers: u32,
    max_walk_miles: f64,
) -> Result<Vec<Itinerary>> {
    let max_buses = max_transfers + 1;
    let depart_earliest = arrive_by_secs.saturating_sub(max_minutes * 60).max(now_secs);

    // Stops near origin
    let origin_stops = stops_within(gtfs, origin, max_walk_miles);
    if origin_stops.is_empty() {
        anyhow::bail!("No bus stops found within {max_walk_miles} mile(s) of origin.");
    }

    // Stops near destination with their walk distances
    let dest_stops: HashMap<String, f64> = stops_within(gtfs, destination, max_walk_miles)
        .into_iter()
        .map(|(id, dist)| (id, dist))
        .collect();
    if dest_stops.is_empty() {
        anyhow::bail!("No bus stops found within {max_walk_miles} mile(s) of destination.");
    }

    // best[stop_id][buses_taken] = best arrive_secs seen so far
    let mut best: HashMap<(String, u32), u32> = HashMap::new();
    let mut heap: BinaryHeap<State> = BinaryHeap::new();

    for (stop_id, walk_dist) in &origin_stops {
        let walk_dur = walk_secs(*walk_dist);
        let arrive_at_stop = depart_earliest + walk_dur;
        if arrive_at_stop > arrive_by_secs {
            continue;
        }
        let stop_name = stop_name(gtfs, stop_id);
        let state = State {
            arrive_secs: arrive_at_stop,
            stop_id: stop_id.clone(),
            buses_taken: 0,
            depart_secs: depart_earliest,
            legs: vec![Leg::Walk(WalkLeg {
                from_name: "origin".to_string(),
                to_name: stop_name,
                distance_miles: *walk_dist,
                duration_secs: walk_dur,
            })],
        };
        let key = (stop_id.clone(), 0u32);
        if arrive_at_stop < *best.get(&key).unwrap_or(&u32::MAX) {
            best.insert(key, arrive_at_stop);
            heap.push(state);
        }
    }

    // Collect more candidates than we'll return so sorting can pick the best.
    const COLLECT_RESULTS: usize = MAX_RESULTS * 4;
    let mut results: Vec<Itinerary> = Vec::new();
    // Fingerprints of already-found results to avoid near-duplicates.
    // Key: sequence of (route_name, from_stop, depart_secs) for all bus legs.
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    while let Some(state) = heap.pop() {
        if state.arrive_secs > arrive_by_secs {
            continue;
        }

        // Prune: only keep if still best for this (stop, buses_taken)
        let key = (state.stop_id.clone(), state.buses_taken);
        if *best.get(&key).unwrap_or(&u32::MAX) < state.arrive_secs {
            continue;
        }

        // Check if we can walk to destination from this stop.
        // Only record a result if at least one bus has been taken.
        if state.buses_taken > 0 {
            if let Some(&walk_dist) = dest_stops.get(&state.stop_id) {
                let walk_dur = walk_secs(walk_dist);
                let arrive_dest = state.arrive_secs + walk_dur;
                if arrive_dest <= arrive_by_secs {
                    let total = arrive_dest - state.depart_secs;
                    if total <= max_minutes * 60 {
                        let mut legs = state.legs.clone();
                        legs.push(Leg::Walk(WalkLeg {
                            from_name: stop_name(gtfs, &state.stop_id),
                            to_name: "destination".to_string(),
                            distance_miles: walk_dist,
                            duration_secs: walk_dur,
                        }));
                        let fingerprint = legs.iter().filter_map(|l| match l {
                            Leg::Bus(b) => Some(format!("{}|{}|{}", b.route_name, b.from_stop, b.depart_secs)),
                            _ => None,
                        }).collect::<Vec<_>>().join(",");

                        if seen.contains(&fingerprint) {
                            continue;
                        }
                        seen.insert(fingerprint);

                        results.push(Itinerary {
                            legs,
                            depart_secs: state.depart_secs,
                            arrive_secs: arrive_dest,
                        });
                        if results.len() >= COLLECT_RESULTS {
                            break;
                        }
                    }
                }
            }
        }

        if state.buses_taken >= max_buses {
            continue;
        }

        // Board trips from this stop
        if let Some(departures) = gtfs.stop_departures.get(&state.stop_id) {
            // Find first departure at or after our arrival
            let start = departures.partition_point(|(dep, _)| *dep < state.arrive_secs);
            let end = (start + MAX_TRIPS_PER_STOP).min(departures.len());

            for (dep_secs, trip_id) in &departures[start..end] {
                let Some(stops) = gtfs.trip_stops.get(trip_id) else { continue };

                // Find our boarding position in the trip
                let board_pos = match stops.iter().position(|s| s.stop_id == state.stop_id) {
                    Some(p) => p,
                    None => continue,
                };

                let route_name = gtfs.route_name(trip_id).to_string();
                let board_stop_name = stop_name(gtfs, &state.stop_id);

                // Follow trip to each subsequent stop
                for alight_pos in (board_pos + 1)..stops.len() {
                    let alight = &stops[alight_pos];
                    if alight.arrival_secs > arrive_by_secs {
                        break;
                    }

                    let mut legs = state.legs.clone();
                    legs.push(Leg::Bus(BusLeg {
                        route_name: route_name.clone(),
                        from_stop: board_stop_name.clone(),
                        to_stop: stop_name(gtfs, &alight.stop_id),
                        depart_secs: *dep_secs,
                        arrive_secs: alight.arrival_secs,
                        stop_count: (alight_pos - board_pos) as u32,
                    }));

                    let new_buses = state.buses_taken + 1;
                    let key = (alight.stop_id.clone(), new_buses);
                    if alight.arrival_secs < *best.get(&key).unwrap_or(&u32::MAX) {
                        best.insert(key, alight.arrival_secs);
                        heap.push(State {
                            arrive_secs: alight.arrival_secs,
                            stop_id: alight.stop_id.clone(),
                            buses_taken: new_buses,
                            depart_secs: state.depart_secs,
                            legs,
                        });
                    }
                }
            }
        }

        // Walking transfers to nearby stops (doesn't consume a bus slot)
        for (nearby_id, walk_dist) in stops_within(gtfs, stop_coord(gtfs, &state.stop_id), TRANSFER_WALK_MILES) {
            if nearby_id == state.stop_id || walk_dist < MIN_TRANSFER_WALK_MILES {
                continue;
            }
            let walk_dur = walk_secs(walk_dist);
            let arrive_nearby = state.arrive_secs + walk_dur;
            if arrive_nearby > arrive_by_secs {
                continue;
            }
            let key = (nearby_id.clone(), state.buses_taken);
            if arrive_nearby < *best.get(&key).unwrap_or(&u32::MAX) {
                best.insert(key, arrive_nearby);
                let mut legs = state.legs.clone();
                legs.push(Leg::Walk(WalkLeg {
                    from_name: stop_name(gtfs, &state.stop_id),
                    to_name: stop_name(gtfs, &nearby_id),
                    distance_miles: walk_dist,
                    duration_secs: walk_dur,
                }));
                heap.push(State {
                    arrive_secs: arrive_nearby,
                    stop_id: nearby_id,
                    buses_taken: state.buses_taken,
                    depart_secs: state.depart_secs,
                    legs,
                });
            }
        }
    }

    // Sort: fewest walking seconds first, then earliest arrival
    results.sort_by_key(|it| {
        let walk_secs_total: u32 = it.legs.iter().filter_map(|l| match l {
            Leg::Walk(w) => Some(w.duration_secs),
            _ => None,
        }).sum();
        (walk_secs_total, it.arrive_secs)
    });
    results.truncate(MAX_RESULTS);

    Ok(results)
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn stops_within(gtfs: &GtfsData, coord: Coord, max_miles: f64) -> Vec<(String, f64)> {
    gtfs.stops
        .values()
        .filter_map(|s| {
            let d = distance_miles(coord.lat, coord.lon, s.lat, s.lon);
            if d <= max_miles { Some((s.id.clone(), d)) } else { None }
        })
        .collect()
}

fn stop_coord(gtfs: &GtfsData, stop_id: &str) -> Coord {
    gtfs.stops.get(stop_id).map(|s| Coord { lat: s.lat, lon: s.lon })
        .unwrap_or(Coord { lat: 0.0, lon: 0.0 })
}

fn stop_name(gtfs: &GtfsData, stop_id: &str) -> String {
    gtfs.stops.get(stop_id).map(|s| s.name.clone()).unwrap_or_else(|| stop_id.to_string())
}
