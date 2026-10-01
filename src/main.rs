//! This project will build a decision tree to try to fully solve Wordle.

mod solver;
mod words;
mod tree_output;
mod tree_input;
mod sampling;

pub use solver::DecisionTree;

pub use words::{LetterResult, Result, WORDS, Word};

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

    /// Random percentage of WORDS used for both guesses and targets (0 < P <= 100)
    #[arg(long, value_name = "PERCENT", default_value = "100",
        value_parser = sampling::parse_percent)]
    sample_percent: f64,

    /// Write the winning decision tree as JSON (replaces an existing file)
    #[arg(short = 'o', long, value_name = "FILE")]
    output: Option<std::path::PathBuf>,

    /// Load a saved JSON decision tree instead of running the solver
    #[arg(long, value_name = "FILE", conflicts_with_all = ["sample_percent", "workers"])]
    load_tree: Option<std::path::PathBuf>,

    /// Interactively solve a puzzle using the built or loaded decision tree
    #[arg(long)]
    interactive: bool,
}

fn main() -> std::io::Result<()> {
    let args = Args::parse();
    let solution = if let Some(path) = &args.load_tree {
        let load = || -> std::io::Result<DecisionTree> {
            DecisionTree::read_json(std::io::BufReader::new(std::fs::File::open(path)?))
        };
        let tree = load().map_err(|error| std::io::Error::new(
            error.kind(), format!("could not load {}: {error}", path.display()),
        ))?;
        println!("Loaded decision tree from {}.", path.display());
        Some(tree)
    } else {
        let workers = args.workers.get();
        println!("Using {workers} minimax workers.");

        println!("Loaded {} five-letter words.", WORDS.len());
        // Let the solver choose every guess, including the opening word.
        use rand::SeedableRng;
        use std::hash::BuildHasher;
        // RandomState supplies a fresh randomized seed without an extra OS-RNG dependency.
        let seed = std::collections::hash_map::RandomState::new().hash_one(());
        let mut rng = rand::rngs::SmallRng::seed_from_u64(seed);
        let words = sampling::sample_words(WORDS, args.sample_percent, &mut rng);
        println!("Using {} words ({}%) as both guesses and targets.", words.len(), args.sample_percent);
        println!("Searching from an empty game state with a {MAX_GUESSES}-guess budget.");
        solver::MinimaxSolver::new(workers).decision_tree(&words, &words, MAX_GUESSES)
    };

    match solution {
        Some(tree) => {
            println!("Strategy: worst case {} total guesses.", tree.worst_case_guesses());
            if let Some(word) = tree.guess() {
                println!("Opening word: {}.", word.as_str());
            } else {
                println!("The tree is already solved.");
            }
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
            if args.interactive {
                // Walk the user through solving a Wordle puzzle using the decision tree:
                interpret_tree(&tree)?;
            }
        }
        None => println!(
            "No strategy guarantees solving every selected target within {MAX_GUESSES} total guesses."
        ),
    }
    Ok(())
}

/// Follow a decision tree using feedback entered on standard input.
fn interpret_tree(tree: &DecisionTree) -> std::io::Result<()> {
    use std::io::{BufRead, Write};
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    let mut line = String::new();
    let mut current = tree;
    let mut guesses = 0;
    while let Some(word) = current.guess() {
        write!(output,
            "Guess {}: {}. Enter five results (_ = grey, + = gold, * = green), or q to quit: ",
            guesses + 1, word.as_str(),
        )?;
        output.flush()?;
        line.clear();
        if input.read_line(&mut line)? == 0 {
            writeln!(output)?;
            return Ok(());
        }
        let feedback = line.trim();
        if feedback.eq_ignore_ascii_case("q") {
            return Ok(());
        }
        if feedback.len() != 5
            || !feedback.bytes().all(|symbol| matches!(symbol, b'_' | b'+' | b'*'))
        {
            writeln!(output, "Enter exactly five symbols using _, +, and *.")?;
            continue;
        }
        let letters = std::array::from_fn(|i| match feedback.as_bytes()[i] {
            b'+' => LetterResult::Gold,
            b'*' => LetterResult::Green,
            _ => LetterResult::Grey,
        });
        match current.after_result(Result::new(letters)) {
            Some(next) => {
                current = next;
                guesses += 1;
            }
            None => writeln!(output,
                "That result is not possible in this decision tree. Check the feedback and try again.",
            )?,
        }
    }
    writeln!(output, "Solved in {guesses} guesses!")?;
    Ok(())
}

#[cfg(test)]
mod tests;
