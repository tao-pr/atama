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

# Premier League form

Finds the results of the last Premier League matches, and writes a short
report on the form of each team that won one of them. A `websearch` action
searches the web with Tavily. Then two Claude Managed Agent steps
(`agent:claude-agent`) read only the search results: the agents have no
tools, so they cannot search or fetch pages themselves, and they use few
tokens. Needs `TAVILY_API_KEY` and `CLAUDE_API_KEY`. Every file goes into
`./output/claude_agent/`.

This job is a sample of the `claude-agent` agent type. The first run
creates the agent of each step and the default environment, and keeps
their IDs in `.atama/claude-lock.json`. The next runs use them again. Each
agent step stops at 10 minutes or at $0.50. With `--debug`, Atama writes
the link of each session in the Claude Console to stderr.

```bash
cargo run -- --dry-run jobs/sample_claude_agent.md
cargo run -- --debug jobs/sample_claude_agent.md
```

## Prepare

- actions:
  - bash mkdir -p ./output/claude_agent

## Search

Tavily searches the last Premier League results. The output is the Tavily
response JSON, with a short text of each page.

- actions:
  - websearch max_results=10 topic="Premier League results this week final scores"
  - write_file ./output/claude_agent/search.json

## Last Results

The agent reads the search results, and replies with JSON that matches the
`matches` schema: 10 matches at most, with the score and the winner of
each.

- actions:
  - read_file ./output/claude_agent/search.json
  - agent:claude-agent
    - model=claude
    - output_schema=matches
    - max_time=10m
    - max_cost_usd=0.50
    - task=The input is the JSON response of a web search for the latest
      Premier League results. Use only this input. List the most recent
      finished Premier League matches that it has, 10 matches at most, the
      newest first. For each match, give the date, the two teams, the goals
      of each team, the winner, or null for a draw, and the URL of the
      search result that has the score. Leave out a match whose score is not
      in the input.
  - write_file ./output/claude_agent/matches.json

## Form of the Winners

The agent reads the results of the last step and the search results. For
each team that won one of the matches, it writes two sentences at most on
its form. The report is Markdown.

- actions:
  - bash cat ./output/claude_agent/matches.json ./output/claude_agent/search.json
  - agent:claude-agent
    - model=claude
    - max_time=10m
    - max_cost_usd=0.50
    - task=The input has two JSON values. The first is a list of the most
      recent Premier League results. The second is the web search response
      that the results come from. Use only this input. Take each team that
      won one of the matches. For each team, describe its form in two
      sentences at most, from what the input says about its recent results.
      Reply with Markdown only: a `# Form of the winners` heading, then a
      `## <team>` heading and the sentences for each team, in the order of
      the results. Do not add other text.
  - write_file ./output/claude_agent/winners_form.md
