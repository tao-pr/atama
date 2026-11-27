```config
claude: model=claude-opus-5-5, base_url=https://api.anthropic.com/v1, api_key=CLAUDE_API_KEY, effort=high
```

```json:matches
{
  "type": "object",
  "properties": {
    "matches": {
      "type": "array",
      "maxItems": 10,
      "items": {
        "type": "object",
        "properties": {
          "date": {
            "type": "string",
            "description": "The date of the match, as YYYY-MM-DD."
          },
          "home_team": { "type": "string" },
          "away_team": { "type": "string" },
          "home_goals": { "type": "integer", "minimum": 0 },
          "away_goals": { "type": "integer", "minimum": 0 },
          "winner": {
            "type": ["string", "null"],
            "description": "The name of the team that won, or null for a draw."
          },
          "source_url": { "type": "string" }
        },
        "required": [
          "date",
          "home_team",
          "away_team",
          "home_goals",
          "away_goals",
          "winner",
          "source_url"
        ],
        "additionalProperties": false
      }
    }
  },
  "required": ["matches"],
  "additionalProperties": false
}
```

```claude-env:offline
networking: limited
allow_package_managers: false
```

# Premier League form

Finds the results of the last Premier League matches, and writes a short
report on the form of each team that won one of them. A `websearch` action
searches the web with Tavily. Then two Claude Managed Agent steps
(`agent:claude-agent`) read only the search results. The agents have no
native tools, so they cannot search or fetch pages themselves, and they use
few tokens. The first agent has the local `write_file` tool, and a success
check makes sure that it wrote a valid JSON file. The second agent reads
the search results with the local `read_file` tool. Needs `TAVILY_API_KEY`
and `CLAUDE_API_KEY`. Every file goes into `./output/claude_agent/`.

This job is a sample of the `claude-agent` agent type. The first run
creates the agent of each step and the default environment, and keeps
their IDs in `.atama/claude-lock.json`. The next runs use them again. Each
agent step stops at 10 minutes, at $0.50 or at its token budget. With `--debug`, Atama writes
the link of each session in the Claude Console to stderr. Without
`--debug`, Atama does not show the link.

```bash
cargo run -- --dry-run jobs/sample_claude_agent.md
cargo run -- --debug jobs/sample_claude_agent.md
```

## Prepare

The step removes the `matches.json` of an earlier run, so that the success
check of "Last Results" cannot pass on an old file.

- actions:
  - bash mkdir -p ./output/claude_agent
  - bash rm -f ./output/claude_agent/matches.json

## Search

Tavily searches the last Premier League results. The output is the Tavily
response JSON, with a short text of each page.

- actions:
  - websearch max_results=10 topic="Premier League results this week final scores"
  - write_file ./output/claude_agent/search.json

## Last Results

The agent reads the search results, writes `matches.json` with the local
`write_file` tool, and replies with the same JSON. The reply must match the
`matches` schema: 10 matches at most, with the score and the winner of
each. Then the success check runs on this machine. It makes sure that the
file exists, is not empty, and is valid JSON. If the check fails, the
agent gets the error and tries again, 3 turns at most.

`name=` gives the agent object a fixed name in the Claude Console and in
the lock file. `system=` is the system prompt of the stored agent.
`autonomy=no_ask` lets the agent write without a prompt on the terminal.
`protect=` stops it from writing over the search results. `max_turns=3`
is the default, written here to show the limit of the success check.

- actions:
  - read_file ./output/claude_agent/search.json
  - agent:claude-agent
    - model=claude
    - name=atama-sample-last-results
    - system=You read football results from web search data. You never
      guess a score that the data does not give.
    - output_schema=matches
    - local_tools=write_file
    - autonomy=no_ask
    - protect=./output/claude_agent/search.json
    - success_check=test -s ./output/claude_agent/matches.json && python3 -m json.tool ./output/claude_agent/matches.json > /dev/null
    - max_turns=3
    - max_time=10m
    - max_cost_usd=0.50
    - token_budget=200_000
    - task=The input is the JSON response of a web search for the latest
      Premier League results. Use only this input. List the most recent
      finished Premier League matches that it has, 10 matches at most, the
      newest first. For each match, give the date, the two teams, the goals
      of each team, the winner, or null for a draw, and the URL of the
      search result that has the score. Leave out a match whose score is not
      in the input. Write the JSON value to
      ./output/claude_agent/matches.json with the write_file tool. Then
      reply with the same JSON value.

## Form of the Winners

The input of the agent is the output of the last step, the `matches`
JSON. The agent finds the search results with the local `list_files` tool,
and reads them with the local `read_file` tool. For each team that won one
of the matches, it writes two sentences at most on its form. The report is
Markdown.

`autonomy=ask_if_risky` lets the read-only tools run without approval.
`on_ask=deny` denies any other call without a prompt on the terminal.
`environment=offline` selects the ```` ```claude-env:offline ```` block,
because the agent needs no network in its cloud container.

- actions:
  - agent:claude-agent
    - model=claude
    - environment=offline
    - local_tools=list_files, read_file
    - autonomy=ask_if_risky
    - on_ask=deny
    - max_time=10m
    - max_cost_usd=0.50
    - token_budget=200_000
    - task=The input is a JSON list of the most recent Premier League
      results. List the files in ./output/claude_agent/ with the
      list_files tool, and read search.json with the read_file tool. It
      is the web search response that the results come from. Use only the
      input and this file. Take each team that won one of the matches.
      For each team, describe its form in two sentences at most, from
      what the input and the file say about its recent results.
      Reply with Markdown only: a `# Form of the winners` heading, then a
      `## <team>` heading and the sentences for each team, in the order of
      the results. Do not add other text.
  - write_file ./output/claude_agent/winners_form.md
