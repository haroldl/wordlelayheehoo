//! Human-readable decision-tree JSON: "_" grey, "+" gold, "*" green.
//! Only validated ASCII words and fixed feedback symbols are emitted as strings;
//! none can contain a JSON quote, backslash, or control character.

use std::io::{self, Write};

use crate::{DecisionTree, LetterResult};

impl DecisionTree {
    /// Writes indented JSON with sorted feedback branches and a trailing newline.
    /// Guess nodes have "guess" and "branches" fields; leaves are {"solved": true}.
    /// Feedback keys use "_" for grey, "+" for gold, and "*" for green.
    pub fn write_json(&self, writer: &mut impl Write) -> io::Result<()> {
        self.write_json_node(writer, 0)?;
        writeln!(writer)
    }

    fn write_json_node(&self, writer: &mut impl Write, indent: usize) -> io::Result<()> {
        match self {
            Self::Solved => write!(writer, "{{\"solved\": true}}"),
            Self::Guess { word, branches } => {
                writeln!(writer, "{{")?;
                writeln!(writer, "{:width$}\"guess\": \"{}\",", "", word.as_str(), width = indent + 2)?;
                write!(writer, "{:width$}\"branches\": {{", "", width = indent + 2)?;
                let mut branches: Vec<_> = branches.iter().map(|(result, next)| {
                    let pattern: String = result.as_slice().iter().map(|letter| match letter {
                        LetterResult::Grey => '_',
                        LetterResult::Gold => '+',
                        LetterResult::Green => '*',
                    }).collect();
                    (pattern, next)
                }).collect();
                branches.sort_unstable_by(|a, b| a.0.cmp(&b.0));
                for (index, (pattern, next)) in branches.iter().enumerate() {
                    if index > 0 {
                        write!(writer, ",")?;
                    }
                    writeln!(writer)?;
                    write!(writer, "{:width$}\"{pattern}\": ", "", width = indent + 4)?;
                    next.write_json_node(writer, indent + 4)?;
                }
                if !branches.is_empty() {
                    writeln!(writer)?;
                    write!(writer, "{:width$}", "", width = indent + 2)?;
                }
                writeln!(writer, "}}")?;
                write!(writer, "{:width$}}}", "", width = indent)
            }
        }
    }
}
