//! Read saved decision trees with serde_json and validate the tree schema.

use std::io::{self, Read};
use serde_json::Value;
use crate::{DecisionTree, LetterResult, Result as Feedback, Word};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn tree(value: Value, played: &mut Vec<Word>) -> io::Result<DecisionTree> {
    let Value::Object(mut fields) = value else { return Err(invalid("tree node must be an object")); };
    let guess = fields.remove("guess");
    let solved = fields.remove("solved");
    let branches = fields.remove("branches");
    if !fields.is_empty() { return Err(invalid("unknown tree node field")); }
    let word = match guess {
        Some(Value::String(text)) => Some(Word::new(&text).ok_or_else(|| invalid("guess must be five ASCII letters"))?),
        None => None,
        _ => return Err(invalid("guess must be a string")),
    };
    if let Some(solved) = solved {
        if !matches!(solved, Value::Bool(true)) || branches.is_some() {
            return Err(invalid("solved must be true and cannot coexist with branches"));
        }
        return match word {
            None => Ok(DecisionTree::Solved),
            Some(word) => {
                if played.contains(&word) { return Err(invalid("repeated guess on a tree path")); }
                Ok(DecisionTree::Guess {
                    word,
                    branches: [(Feedback::new([LetterResult::Green; 5]), DecisionTree::Solved)].into(),
                })
            }
        };
    }
    let word = word.ok_or_else(|| invalid("unsolved node needs a guess"))?;
    if played.contains(&word) { return Err(invalid("repeated guess on a tree path")); }
    let Some(Value::Object(branches)) = branches else { return Err(invalid("guess needs branches or solved: true")); };
    if branches.is_empty() { return Err(invalid("branches must not be empty")); }
    played.push(word);
    let mut decoded = std::collections::HashMap::new();
    for (pattern, next) in branches {
        let bytes: [u8; 5] = pattern.as_bytes().try_into().map_err(|_| invalid("feedback must have five symbols"))?;
        let mut letters = [LetterResult::Grey; 5];
        for (letter, symbol) in letters.iter_mut().zip(bytes) {
            *letter = match symbol {
                b'_' => LetterResult::Grey, b'+' => LetterResult::Gold, b'*' => LetterResult::Green,
                _ => return Err(invalid("feedback symbols must be _, +, or *")),
            };
        }
        let child = tree(next, played)?;
        if (letters == [LetterResult::Green; 5]) != matches!(child, DecisionTree::Solved) {
            return Err(invalid("only all-green feedback may lead directly to solved"));
        }
        decoded.insert(Feedback::new(letters), child);
    }
    played.pop();
    Ok(DecisionTree::Guess { word, branches: decoded })
}

impl DecisionTree {
    /// Loads both compact final guesses and the older explicit all-green branch.
    /// Checks structure and repeated guesses, but does not re-prove dictionary coverage.
    pub fn read_json(reader: impl Read) -> io::Result<Self> {
        let value: Value = serde_json::from_reader(reader).map_err(|error| {
            io::Error::new(error.io_error_kind().unwrap_or(io::ErrorKind::InvalidData), error)
        })?;
        tree(value, &mut Vec::new())
    }
}
