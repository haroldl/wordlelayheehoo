//! This project will build a decision tree to try to fully solve Wordle.

mod game_state;
mod solver;
mod words;

pub use solver::DecisionTree;

pub use words::{LetterResult, Result, WORDS, Word};

pub use game_state::{GameState, possible_states_after_guesses};
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
}

fn main() {
    let args = Args::parse();
    let workers = args.workers.get();
    println!("Using {workers} minimax workers.");

    println!("Loaded {} five-letter words.", WORDS.len());
    // Fixed opening for faster development; the model also supports an empty state.
    let first_guess = Word::new("arose").unwrap();
    // let second_guess = Word::new("unlit").unwrap();
    // let game_states = possible_states_after_guesses(&[first_guess, second_guess]);
    // println!(
    //     "{} possible game states after guessing {} and {}.",
    //     game_states.len(),
    //     first_guess.as_str(),
    //     second_guess.as_str()
    // );
    let game_states = possible_states_after_guesses(&[first_guess]);
    println!(
        "{} possible game states after guessing only {}.",
        game_states.len(),
        first_guess.as_str()
    );

    let max_guesses = max_minimax_guesses(&game_states, workers);

    match max_guesses {
        Some(count) => println!("Worst case: {count} additional guesses after the opening."),
        None => println!(
            "At least one state cannot be guaranteed solved within {MAX_GUESSES} total guesses."
        ),
    }
}

#[cfg(test)]
mod tests;
