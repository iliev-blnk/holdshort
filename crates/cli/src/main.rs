//! `holdshort`: backfill, compact and METAR-history commands.

mod backfill;
mod compact;
mod dates;
mod filter;
mod history_fetch;
mod parquet_out;
mod store;
mod weather;

use std::process::ExitCode;

use clap::{Parser, Subcommand};

use chrono::Datelike;

use crate::backfill::{Outcome, backfill_day};
use crate::store::Store;

pub const UA: &str = "holdshort/0.1 (+https://github.com/iliev-blnk/holdshort)";

#[derive(Parser)]
#[command(name = "holdshort", about = "IST airspace ingest tools")]
struct Cli {
    /// Local data directory (history, live chunks, Parquet, weather).
    #[arg(long, global = true, default_value = "data")]
    data: String,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Stream adsb.lol history tars and keep the LTFM area.
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
    /// Fetch LTFM METAR history from the Iowa Mesonet archive.
    MetarHistory { range: String },
}

fn run(cli: Cli) -> anyhow::Result<bool> {
    let store = Store::local(&cli.data)?;
    let mut all_ok = true;
    match cli.cmd {
        Cmd::Backfill { range, force } => {
            for day in dates::parse_range(&range)? {
                let started = std::time::Instant::now();
                match backfill_day(&store, day, force, || history_fetch::open_day(day)) {
                    Ok(Outcome::Skipped) => println!("{day} skipped (exists)"),
                    Ok(Outcome::Done(s)) => println!(
                        "{day} ok: {} traces, {} obs kept, {} bad, {:.0}s",
                        s.traces,
                        s.kept_obs,
                        s.bad_traces,
                        started.elapsed().as_secs_f64()
                    ),
                    Err(e) => {
                        all_ok = false;
                        println!("{day} FAILED: {e:#}");
                    }
                }
            }
        }
        Cmd::Compact { range, force } => {
            for day in dates::parse_range(&range)? {
                match compact::compact_day(&store, day, force) {
                    Ok(s) => println!(
                        "{day} ok: {} obs, {} polls, {} live chunks, history: {}",
                        s.obs,
                        s.polls,
                        s.live_chunks,
                        if s.had_history { "yes" } else { "no" }
                    ),
                    Err(e) => {
                        all_ok = false;
                        println!("{day} FAILED: {e:#}");
                    }
                }
            }
        }
        Cmd::MetarHistory { range } => {
            let days = dates::parse_range(&range)?;
            // One IEM request per calendar month in the range.
            for month in days.chunk_by(|a, b| (a.year(), a.month()) == (b.year(), b.month())) {
                let (from, to) = (month[0], month[month.len() - 1]);
                match weather::fetch_iem(from, to).and_then(|m| {
                    let n = m.len();
                    weather::store_metars(&store, m).map(|_| n)
                }) {
                    Ok(n) => println!("{from}..{to} ok: {n} reports"),
                    Err(e) => {
                        all_ok = false;
                        println!("{from}..{to} FAILED: {e:#}");
                    }
                }
            }
        }
    }
    Ok(all_ok)
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("holdshort: {e:#}");
            ExitCode::FAILURE
        }
    }
}
