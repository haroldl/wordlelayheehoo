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

To solve a puzzle later using the saved tree:

```bash
cargo run --release -- --load-tree tree.json --interactive
```

Interaction is disabled by default, so building and saving a tree exits without prompting. Add `--interactive` to either a build or load command to enter feedback using `_` (grey), `+` (gold), and `*` (green), or `q` to quit.

## Options

| Option | Default | Meaning |
| --- | --- | --- |
| `--sample-percent PERCENT` | `100` | Percentage of dictionary entries used for both guesses and targets. Must be greater than 0 and at most 100; decimals are supported. |
| `-w`, `--workers N` | `10` | Number of solver workers; must be positive. |
| `-o`, `--output FILE` | No file | Write the winning decision tree as indented JSON, replacing an existing file. |
| `--load-tree FILE` | No file | Load a saved strategy instead of searching. Conflicts with explicit worker and sampling options. |
| `--interactive` | `false` | Prompt for feedback to solve a puzzle using the built or loaded tree. |
| `-h`, `--help` | | Show command-line help. |

## Random sampling

A sample is selected once per run, without replacement, and remains fixed throughout the search. The entry count is rounded up: `ceil(WORDS.len() * percent / 100)`. A positive percentage retains at least one entry from a nonempty dictionary. Existing duplicate dictionary entries are treated as separate entries. At 100%, the full dictionary is used without shuffling.

For a smaller development run:

```bash
cargo run --release -- --sample-percent 0.1 -o small-tree.json
```

Each sampled run receives a fresh random seed; there is currently no command-line seed option. A successful strategy guarantees a win only for the selected targets, using guesses from that same sample. It does not establish that the full dictionary can be solved within six guesses.

## Load a saved strategy

```bash
cargo run --release -- --load-tree tree.json
```

This reports the saved opening word and worst-case length without starting solver workers or sampling the dictionary. To write it back in the current format:

```bash
cargo run --release -- --load-tree tree.json -o copy.json
```

Reading and writing use `serde_json`. Both compact final guesses and older explicit `*****` branches are accepted. Loading validates the tree structure, words, feedback symbols, and repeated guesses along a path; it does not re-run the solver to prove dictionary coverage. Invalid or unreadable files produce an error rather than starting a new search.

## Reading the decision tree

Each decision node contains a `guess` word and a `branches` object. A final guess whose only outcome is all green has `"solved": true` in place of `branches`. After making the guess, follow the key matching the observed feedback, in letter order:

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
      "solved": true
    }
  }
}
```

Only reachable feedback outcomes appear. A `{"solved": true}` leaf means the target has been guessed correctly. A node containing both `guess` and `"solved": true` tells you to play that final word to win. The file is written only after a winning strategy is found; if no strategy is found, an existing output file is left unchanged.

## Tests

```bash
cargo test
```
