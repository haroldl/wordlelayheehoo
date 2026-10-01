//! Human-readable decision-tree JSON, serialized with serde_json.

use std::io::{self, Write};
use serde_json::{Value, json};
use crate::{DecisionTree, LetterResult, Result};

impl DecisionTree {
    /// Writes indented JSON with sorted feedback branches and a trailing newline.
    /// Final guesses use {"guess": "...", "solved": true}; solved leaves omit guess.
    /// Feedback keys use "_" for grey, "+" for gold, and "*" for green.
    pub fn write_json(&self, writer: &mut impl Write) -> io::Result<()> {
        serde_json::to_writer_pretty(&mut *writer, &self.json_value()).map_err(|error| {
            io::Error::new(error.io_error_kind().unwrap_or(io::ErrorKind::InvalidData), error)
        })?;
        writeln!(writer)
    }

    fn json_value(&self) -> Value {
        match self {
            Self::Solved => json!({ "solved": true }),
            Self::Guess { word, branches } => {
                if branches.len() == 1 && branches.iter().any(|(result, next)| {
                    *result == Result::ALL_GREEN
                        && matches!(next, Self::Solved)
                }) {
                    return json!({ "guess": word.as_str(), "solved": true });
                }
                let branches: serde_json::Map<String, Value> = branches.iter().map(|(result, next)| {
                    let pattern: String = result.to_letters().into_iter().map(|letter| match letter {
                        LetterResult::Grey => '_',
                        LetterResult::Gold => '+',
                        LetterResult::Green => '*',
                    }).collect();
                    (pattern, next.json_value())
                }).collect();
                json!({ "guess": word.as_str(), "branches": branches })
            }
        }
    }
}
