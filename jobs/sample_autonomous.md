```config
claude-haiku: model=claude-haiku-5-5, base_url=https://api.anthropic.com/v1, api_key=CLAUDE_API_KEY
```

```loop:report
max_iterations: 4
max_time: 10m
token_budget: 200000
success_check: python3 ./output/autonomous/check_report.py ./output/autonomous/report.json
task:
  - Write ./output/autonomous/report.json with the release cycles of python, nodejs and postgresql that are not end of life today.
  - Find an API with JSON end-of-life data in the search results in ./output/autonomous/search.json.
  - Download the JSON of each product into ./output/autonomous/data/.
  - Write ./output/autonomous/build_report.py, which reads the downloaded files and writes the report. Run it.
  - Report format, as JSON. Top level keys generated (today, YYYY-MM-DD) and products. products maps each product to a list of cycles, in the order of the source. Each cycle has the keys cycle, latest, eol (the date of the source, or false) and days_left (the days from today to eol, or null if eol is false).
guardrails:
  - Do not change or delete ./output/autonomous/check_report.py.
  - Write files only in ./output/autonomous/.
```

# Autonomous sample

Which release cycles of Python, Node.js and PostgreSQL are still supported?
A `coder` agent writes a checker that knows the answer. Then an
`autonomous` agent searches, downloads and parses the data, and writes a
Python script, until the checker accepts its report. Needs `python3`,
`CLAUDE_API_KEY` and `TAVILY_API_KEY`. The agents run commands with your
permissions. Every file goes into `./output/autonomous/`.

```bash
cargo run -- jobs/sample_autonomous.md
```

## Prepare Dir

- actions:
  - bash rm -rf ./output/autonomous
  - bash mkdir -p ./output/autonomous/data

## Search

- actions:
  - websearch topic="API with JSON end-of-life dates of Python, Node.js and PostgreSQL releases" max_results=5
  - write_file ./output/autonomous/search.json

## Create Success Checker Script

- actions:
  - agent:coder model=claude-haiku task="Write ./output/autonomous/check_report.py. It reads the JSON report in its first argument. For each of python, nodejs and postgresql, it downloads https://endoflife.date/api/<product>.json with urllib, and keeps the cycles whose eol is false or a date after today, in the order of the source. The report must have generated equal to today as YYYY-MM-DD, and products, which maps each product to its cycles. Each cycle has cycle, latest, eol (the date, or false) and days_left (days from today to eol, or null if eol is false). The script exits with 0 if the report matches. Else it prints the first difference and exits with 1."
  - bash test -f ./output/autonomous/check_report.py

## Autonomous Report

- actions:
  - agent:autonomous model=claude-haiku loop=report
