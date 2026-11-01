```config
local-spark2: model=hf.co/XHToken/Spark-X2.6-1.7B-GGUF:Q4_K_M, base_url=http://localhost:11434/v1
```

```json:city_temperature
{
  "type": "object",
  "properties": {
    "city": {
      "type": "string",
      "enum": ["berlin", "munich", "madrid", "moscow", "tokyo"]
    },
    "minTemp": {
      "type": "number",
      "description": "The lowest temperature of today, in degrees Celsius."
    },
    "maxTemp": {
      "type": "number",
      "description": "The highest temperature of today, in degrees Celsius."
    }
  },
  "required": ["city", "minTemp", "maxTemp"],
  "additionalProperties": false
}
```

# City temperatures

Searches the web for the temperatures of today in five cities, and lets a
`summarizer` agent read the lowest and the highest temperature of each city
from its search result. The agent replies with JSON that matches the
`city_temperature` schema, and the job writes the reply for each city to
its own file. This file is a sample of `output_schema=`. Every file goes
into `./output/structured/`.

The ```` ```json:city_temperature ```` block at the start of the file is
an output schema. An agent action selects it with
`output_schema=city_temperature`. Atama sends the schema to the model, and
makes sure that each reply matches it. If a reply does not match, the model
gets the errors and tries again. The `enum` of `city` allows only the five
city names, so each name is also a safe file name.

Prerequisites:

- Ollama on `localhost:11434`, with the model of the `local-spark2`
  profile. To set up Ollama, see `jobs/sample_agents.md`. To download the
  model, run this command:

  ```bash
  ollama pull hf.co/XHToken/Spark-X2.6-1.7B-GGUF:Q4_K_M
  ```

- A Tavily API key, for the `Search` task. Each city is one search, so a
  run uses five searches of Tavily API credits:

  ```bash
  export TAVILY_API_KEY=<your Tavily key>
  ```

- `jq`, to read and write the JSON. To install it, see
  https://jqlang.org/download/.

To run the job:

```bash
cargo run -- jobs/sample_agent_with_structured_output.md
```

To see the requests to the model, the replies, and the schema errors, add
`--debug`:

```bash
cargo run -- --debug jobs/sample_agent_with_structured_output.md 2> debug.log
```

## Prepare

Delete the files of an earlier run, and make the output directory again.

- actions:
  - bash rm -rf ./output/structured
  - bash mkdir -p ./output/structured

## Cities

Print one city on each line. `printf` repeats its format for each argument.

- actions:
  - bash printf %s\n berlin munich madrid moscow tokyo

## Queries

Make one search query from each city, as in "berlin weather today minimum
maximum temperature". The awk program has no spaces, because Atama splits
an action line at its spaces.

- actions:
  - bash awk {print$0,"weather","today","minimum","maximum","temperature"}

## Search

Search the web once for each query. The `websearch` action always searches
its own `topic=` and ignores its input, so this task uses `tavily_search`:
without query words in the action, its query is its input. `foreach` gives
it one query at a time. `include_answer=basic` asks Tavily for a short
answer to the query, which helps the small model. The output is the Tavily
response of each city, one after the other.

- actions:
  - foreach tavily_search max_results=3 include_answer=basic
- if_fail: retry=1

## Shorten results

Keep only the query, the answer, and the title and the content of each
result, so that the input of the model is short. `jq -c` writes the
response of each city on one line, and `foreach` in the next task needs one
item on each line.

- actions:
  - bash jq -c {query,answer,results:[.results[]|{title,content}]}

## Extract temperatures

Run the `summarizer` agent once for each city. `output_schema=city_temperature`
makes the reply one JSON object with `city`, `minTemp` and `maxTemp`.
`thinking=false` turns off the thinking of the model, which is faster on a
CPU. The output is the JSON object of each city, one after the other.

- actions:
  - foreach agent:summarizer model=local-spark2 thinking=false output_schema=city_temperature task="Read the search result of one city. Find the lowest and the highest temperature of today in degrees Celsius. If the result gives degrees Fahrenheit, convert them to Celsius. Write the city name in lowercase."

## Save cities

Write the JSON object of each city to `./output/structured/<city>.json`, for
example `berlin.json`. `jobs/scripts/write_json_by_key.sh` reads the file
name from `city`. It prints the path of each file, one per line.

- actions:
  - bash ./jobs/scripts/write_json_by_key.sh ./output/structured city

## Print cities

Print each file that the job wrote. `xargs` gives the paths from Save
cities to `jq -c .`, which writes each file on one line.

- actions:
  - bash xargs jq -c .
