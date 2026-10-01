//! This project will build a decision tree to try to fully solve Wordle.

mod game_state;
mod solver;
mod words;
mod tree_output;

pub use solver::DecisionTree;

pub use words::{LetterResult, Result, WORDS, Word};

pub use game_state::{GameState, possible_states_after_guesses};
#[cfg(test)]
use game_state::max_minimax_guesses;

use clap::Parser;
use std::num::NonZeroUsize;

/// Maximum number of guesses allowed in a game.
pub const MAX_GUESSES: usize = 6;

/// Default number of minimax workers; override with --workers N.
pub const DEFAULT_SOLVER_WORKERS: usize = 10;

#[derive(Parser)]
#[command(version, about = "Wordle minimax solver")]
struct Args {
    /// Number of solver worker threads
    #[arg(
        short = 'w',
        long,
        default_value_t = NonZeroUsize::new(DEFAULT_SOLVER_WORKERS)
            .expect("default worker count must be positive")
    )]
    workers: NonZeroUsize,

    /// Write the winning decision tree as JSON (replaces an existing file)
    #[arg(short = 'o', long, value_name = "FILE")]
    output: Option<std::path::PathBuf>,
}

fn main() -> std::io::Result<()> {
    let args = Args::parse();
    let workers = args.workers.get();
    println!("Using {workers} minimax workers.");

    println!("Loaded {} five-letter words.", WORDS.len());
    // Let the solver choose every guess, including the opening word.
    let state = GameState::new();
    println!("Searching from an empty game state with a {MAX_GUESSES}-guess budget.");

    match state.decision_tree_with_workers(workers) {
        Some(tree) => {
            println!("Found strategy: worst case {} total guesses.", tree.worst_case_guesses());
            println!("Opening word: {}.", tree.guess().unwrap().as_str());
            if let Some(path) = args.output {
                use std::io::Write;
                let save = || -> std::io::Result<()> {
                    let mut writer = std::io::BufWriter::new(std::fs::File::create(&path)?);
                    tree.write_json(&mut writer)?;
                    writer.flush()
                };
                save().map_err(|error| std::io::Error::new(
                    error.kind(), format!("could not save {}: {error}", path.display()),
                ))?;
                println!("Saved decision tree to {}.", path.display());
            }
        }
        None => println!(
            "No strategy guarantees solving every target within {MAX_GUESSES} total guesses."
        ),
    }
    Ok(())
}

#[cfg(test)]
mod tests;
