//! Choose one dictionary subset per run, without replacement.

use rand::{Rng, seq::SliceRandom};
use crate::Word;

pub(crate) fn parse_percent(text: &str) -> Result<f64, String> {
    let percent: f64 = text.parse().map_err(|_| "expected a percentage in (0, 100]")?;
    if !percent.is_finite() || percent <= 0.0 || percent > 100.0 {
        return Err("percentage must be greater than 0 and at most 100".into());
    }
    Ok(percent)
}

/// Samples entries without replacement, rounding up to retain at least one
/// entry for any positive percentage of a nonempty dictionary.
/// At 100%, preserves the original dictionary and its order.
/// Existing duplicate entries are treated as separate entries.
pub(crate) fn sample_words(words: &[Word], percent: f64, rng: &mut impl Rng) -> Vec<Word> {
    assert!(percent.is_finite() && percent > 0.0 && percent <= 100.0);
    let mut selected = words.to_vec();
    if percent < 100.0 {
        let count = ((percent / 100.0 * words.len() as f64).ceil() as usize)
            .max(1).min(words.len());
        selected.shuffle(rng);
        selected.truncate(count);
    }
    selected
}
