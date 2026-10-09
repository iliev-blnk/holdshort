//! `holdshort`: backfill, live, compact and METAR-history commands.

mod backfill;
mod compact;
mod dates;
mod filter;
mod history_fetch;
mod live;
mod parquet_out;
mod store;
mod weather;

use std::process::ExitCode;

use chrono::Datelike;
use clap::{Parser, Subcommand};
use holdshort_core::airport::{AIRPORTS, Airport, LTFM, by_icao};

use crate::backfill::{Outcome, backfill_day};
use crate::store::Store;

pub const UA: &str = "holdshort/0.1 (+https://github.com/iliev-blnk/holdshort)";

#[derive(Parser)]
#[command(name = "holdshort", about = "Airport airspace ingest tools")]
struct Cli {
    /// Local data directory (one folder per airport).
    #[arg(long, global = true, default_value = "data")]
    data: String,
    /// Airport ICAO code; repeat for several (default LTFM).
    #[arg(long = "airport", global = true, value_parser = parse_airport)]
    airports: Vec<&'static Airport>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Stream adsb.lol history tars (once per day) and keep each airport's circle.
    Backfill {
        /// A date (2026-10-08) or an inclusive range (2026-09-01..2026-09-30).
        range: String,
        /// Redo days that already exist.
        #[arg(long)]
        force: bool,
    },
    /// Merge history + live chunks into Parquet; merge METAR into monthly files.
    Compact {
        range: String,
        #[arg(long)]
        force: bool,
    },
    /// Poll live traffic from this machine into the data dir (Ctrl-C to stop).
    Live,
    /// Fetch METAR history from the Iowa Mesonet archive.
    MetarHistory { range: String },
}

fn parse_airport(s: &str) -> Result<&'static Airport, String> {
    by_icao(s).ok_or_else(|| {
        let known: Vec<&str> = AIRPORTS.iter().map(|a| a.icao).collect();
        format!("unknown airport {s:?}; known: {}", known.join(", "))
    })
}

fn run(cli: Cli) -> anyhow::Result<bool> {
    let store = Store::local(&cli.data)?;
    let airports: Vec<&'static Airport> = if cli.airports.is_empty() {
        vec![&LTFM]
    } else {
        cli.airports
    };
    let mut all_ok = true;
    let mut report = |what: String, r: anyhow::Result<String>| match r {
        Ok(msg) => println!("{what} ok: {msg}"),
        Err(e) => {
            all_ok = false;
            println!("{what} FAILED: {e:#}");
        }
    };
    match cli.cmd {
        Cmd::Backfill { range, force } => {
            for day in dates::parse_range(&range)? {
                let started = std::time::Instant::now();
                let r = backfill_day(&store, &airports, day, force, || {
                    history_fetch::open_day(day)
                })
                .map(|out| match out {
                    Outcome::Skipped => "skipped (exists)".to_string(),
                    Outcome::Done { stats, airports } => {
                        let kept: Vec<String> = airports
                            .iter()
                            .zip(&stats.kept)
                            .map(|(a, n)| format!("{a} {n} obs"))
                            .collect();
                        format!(
                            "{} traces, {} bad, {}, {:.0}s",
                            stats.traces,
                            stats.bad_traces,
                            kept.join(", "),
                            started.elapsed().as_secs_f64()
                        )
                    }
                });
                report(day.to_string(), r);
            }
        }
        Cmd::Compact { range, force } => {
            for day in dates::parse_range(&range)? {
                for a in &airports {
                    let r = compact::compact_day(&store, a, day, force).map(|s| {
                        format!(
                            "{} obs, {} polls, {} live chunks, history: {}",
                            s.obs,
                            s.polls,
                            s.live_chunks,
                            if s.had_history { "yes" } else { "no" }
                        )
                    });
                    report(format!("{} {day}", a.icao), r);
                }
            }
        }
        Cmd::Live => live::run(std::path::Path::new(&cli.data), &airports)?,
        Cmd::MetarHistory { range } => {
            let days = dates::parse_range(&range)?;
            // One IEM request per airport per calendar month in the range.
            for month in days.chunk_by(|a, b| (a.year(), a.month()) == (b.year(), b.month())) {
                let (from, to) = (month[0], month[month.len() - 1]);
                for a in &airports {
                    let r = weather::fetch_iem(a, from, to).and_then(|m| {
                        let n = m.len();
                        weather::store_metars(&store, a, m).map(|_| format!("{n} reports"))
                    });
                    report(format!("{} {from}..{to}", a.icao), r);
                }
            }
        }
    }
    Ok(all_ok)
}

fn main() -> ExitCode {
    live::load_env(std::path::Path::new(".env"));
    match run(Cli::parse()) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("holdshort: {e:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn airport_flag_accepts_known_codes() {
        assert_eq!(parse_airport("lrsv").unwrap().icao, "LRSV");
    }

    #[test]
    fn airport_flag_lists_known_codes_on_error() {
        let err = parse_airport("EGLL").unwrap_err();
        assert!(err.contains("LTFM, LRSV"), "{err}");
    }
}
