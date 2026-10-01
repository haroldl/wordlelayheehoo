# wordlelayheehoo

A Wordle strategy solver in Rust. Starting from an empty game state, it searches for a decision tree that solves every selected target within six guesses, without repeating guesses. It accepts the first proven strategy within that budget; it does not minimize the number of guesses.

Cargo processes `WORD.LST` at build time, keeping five-letter ASCII words and normalizing them to lowercase. The resulting `WORDS` dictionary is embedded in the executable and supplies both allowed guesses and possible targets.

## Run

Use a release build for searches:

```bash
cargo run --release -- --sample-percent 10 -w 10 -o tree.json
```

This selects a random 10% of the dictionary, searches using 10 workers, and saves the winning strategy to `tree.json` if one is found. The program reports the selected word count, opening word, and the strategy's worst-case total guesses.

To search the full dictionary:

```bash
cargo run --release -- -w 20 -o tree.json
```

## Options

| Option | Default | Meaning |
| --- | --- | --- |
| `--sample-percent PERCENT` | `100` | Percentage of dictionary entries used for both guesses and targets. Must be greater than 0 and at most 100; decimals are supported. |
| `-w`, `--workers N` | `10` | Number of solver workers; must be positive. |
| `-o`, `--output FILE` | No file | Write the winning decision tree as indented JSON, replacing an existing file. |
| `-h`, `--help` | | Show command-line help. |

## Random sampling

A sample is selected once per run, without replacement, and remains fixed throughout the search. The entry count is rounded up: `ceil(WORDS.len() * percent / 100)`. A positive percentage retains at least one entry from a nonempty dictionary. Existing duplicate dictionary entries are treated as separate entries. At 100%, the full dictionary is used without shuffling.

For a smaller development run:

```bash
cargo run --release -- --sample-percent 0.1 -o small-tree.json
```

Each sampled run receives a fresh random seed; there is currently no command-line seed option. A successful strategy guarantees a win only for the selected targets, using guesses from that same sample. It does not establish that the full dictionary can be solved within six guesses.

## Reading the decision tree

Each node contains a `guess` word and a `branches` object. After making the guess, follow the key matching the observed feedback, in letter order:

| Symbol | Feedback |
| --- | --- |
| `_` | Grey |
| `+` | Gold |
| `*` | Green |

For example, this complete tree distinguishes `apple` and `ample`:

```json
{
  "guess": "apple",
  "branches": {
    "*****": {"solved": true},
    "*_***": {
      "guess": "ample",
      "branches": {
        "*****": {"solved": true}
      }
    }
  }
}
```

Only reachable feedback outcomes appear. A `{"solved": true}` leaf means the target has been guessed correctly. The file is written only after a winning strategy is found; if no strategy is found, an existing output file is left unchanged.

## Tests

```bash
cargo test
```
