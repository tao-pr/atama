```config
claude-haiku: model=claude-haiku-5-5, base_url=https://api.anthropic.com/v1, api_key=CLAUDE_API_KEY
```

# Autonomous sample

A `coder` agent writes a checker. Then an `autonomous` agent works until
the checker accepts its output. Needs `python3`, and a Claude API key in
`CLAUDE_API_KEY`. The agents run commands with your permissions. Every file
goes into `./output/autonomous/`.

```bash
cargo run -- jobs/sample_autonomous.md
```

## Prepare Dir

- actions:
  - bash mkdir -p ./output/autonomous

## Create Success Checker Script

- actions:
  - agent:coder model=claude-haiku task="Write ./output/autonomous/check_fizzbuzz.py. It reads the file in its first argument. It exits with 0 if the file has exactly 15 lines, and line n is FizzBuzz if n is divisible by 15, Fizz if by 3, Buzz if by 5, else n. Else it prints the first wrong line and exits with 1."
  - bash test -f ./output/autonomous/check_fizzbuzz.py

## Autonomous Fizzbuzz

The checker of the coder checks the output file.

- actions:
  - agent:autonomous model=claude-haiku task="Write ./output/autonomous/fizzbuzz.sh, a bash script that prints FizzBuzz for the numbers 1 to 15. Run it, and save its output to ./output/autonomous/fizzbuzz.txt." success_check="python3 ./output/autonomous/check_fizzbuzz.py ./output/autonomous/fizzbuzz.txt" max_iterations=3 max_time=5m token_budget=50000

