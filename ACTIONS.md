# Actions

This file describes each action of an Atama job and all of its parameters.
For the format of a job file, see [README.md](README.md).

An action is one line in the `- actions:` list of a task. The first word is
the action type. The other words are its parameters.

Each action gets the output of the action before it as its input. The first
action of a task gets the output of the last task that succeeded. If an
action fails, the task fails, and the next actions of the task do not run.

| Action | What it does | Output |
| --- | --- | --- |
| [`bash`](#bash) | Runs a command in bash. | stdout |
| [`bash-sandbox`](#bash-sandbox) | Runs a command in bash in the macOS sandbox. | stdout |
| [`api_call`](#api_call) | Sends an HTTP request. | The response body |
| [`download`](#download) | Downloads a file. | The file path |
| [`websearch`](#websearch) | Searches the web for one topic with Tavily. | The response JSON, unchanged |
| [`tavily_search`](#tavily_search) | Searches the web with Tavily. | The response JSON, unchanged |
| [`agent:<type>`](#agenttype) | Runs a `coder`, `summarizer` or `autonomous` agent. | The final reply of the model |
| [`read_file`](#read_file) | Reads a text file. | The content of the file, unchanged |
| [`list_files`](#list_files) | Lists the files of a directory. | One path per line |
| [`write_file`](#write_file) | Writes the input to a file. | The input, unchanged |
| [`foreach`](#foreach) | Runs an action once for each input line. | All outputs, one after the other |
| [`mock`](#mock) | Does nothing. | The input, unchanged |
| [`mock fail`](#mock-fail) | Always fails. | None |

## Quotes in parameters

Some actions accept a value in double quotes, so that the value can have
spaces, as in `task="Fix the failing test."`. Inside quotes, `\"` is a quote
and `\\` is a backslash. Atama removes the quotes before it uses the value.

These actions accept quotes: `websearch`, `read_file`, `list_files` and
`write_file`. The options of `agent:<type>` also accept quotes, but they do
not need them (see [`agent:<type>`](#agenttype)). The other actions split the
line at each space, and they keep quote characters as they are.

## `bash`

```text
bash <command> [args...]
```

Runs a command in bash, in the directory where you start Atama.

| Parameter | Required | Description |
| --- | --- | --- |
| `<command>` | Yes | The command or script to run, as in `npm`. |
| `[args...]` | No | The arguments of the command, in order. |

The input of the action goes to the stdin of the command. The output is the
stdout of the command. The action fails if the command exits with a status
that is not 0, or if stdout is not UTF-8 text. The error has the stderr of
the command.

Bash does not read the arguments as shell syntax. For example, in
`bash ls | wc -l`, the command `ls` gets `|`, `wc` and `-l` as arguments. To
use pipes or redirection, put the script in a file and run the file, as in
`bash ./scripts/count.sh`.

```markdown
- actions:
  - bash npm ci
  - bash npm test
```

## `bash-sandbox`

```text
bash-sandbox <command> [args...]
```

Runs a command as [`bash`](#bash) does, but inside the macOS sandbox
(`sandbox-exec`). The parameters, the input and the output are the same as
for `bash`.

The sandbox has these limits:

- The command has no network.
- The command can read and write files only in the working directory.
- The command can read the system directories that programs need, such as `/usr` and `/opt/homebrew`.
- The command cannot run `sudo`, `su` or `login`.
- `PATH` is `/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin`.

The home directory is closed, so programs such as `pyenv` shims do not run.
The action stops if Atama runs as root. The profile of the sandbox is in
`src/actor/bash_sandbox.sb`.

`bash-sandbox` works only on macOS. On other systems, the action fails. A
tool that needs a temporary directory outside the working directory can fail
or print warnings.

```markdown
- actions:
  - bash-sandbox python3 ./scripts/check.py
```

## `api_call`

```text
api_call <verb> <url> [payload=<text>] [input_file=<path>] [auth=bearer:<token>]
```

Sends an HTTP request. The output is the response body.

| Parameter | Required | Description |
| --- | --- | --- |
| `<verb>` | Yes | `GET`, `POST`, `PUT`, `DELETE`, `PATCH` or `HEAD`, in uppercase or lowercase. |
| `<url>` | Yes | The URL of the request. |
| `payload=<text>` | No | The request body. The text cannot have spaces. |
| `input_file=<path>` | No | A file whose content is the request body. |
| `auth=bearer:<token>` | No | Sends the header `Authorization: Bearer <token>`. The log shows the token as `***`. |

The request body is `payload=`, else the content of `input_file=`, else the
input of the action. `GET`, `DELETE` and `HEAD` send no body. If the body is
JSON, the `Content-Type` is `application/json`. Else, it is
`text/plain; charset=utf-8`.

A `HEAD` response has no body, so its output is empty. The action fails if
the server responds with a status that is not 2xx. The error has the
response body.

Each option can occur only once. An unknown option, or a word without `=`,
stops the job before it starts.

```markdown
- actions:
  - api_call GET http://localhost:8080/pets auth=bearer:secret-token
  - api_call POST http://localhost:8080/pets input_file=./data/pet.json
```

## `download`

```text
download <url> [mime=<type>] [output_dir=<dir>] [auth=bearer:<token>]
```

Sends a `GET` request and saves the response body to a file. The output is
the path of the file. The action ignores its input.

| Parameter | Required | Description |
| --- | --- | --- |
| `<url>` | Yes | The URL of the file. |
| `mime=<type>` | No | Sends the header `Accept: <type>`, as in `mime=application/pdf`. |
| `output_dir=<dir>` | No | The directory of the file. The default is `atama/` in the temporary directory of the system. Atama makes the directory if it does not exist. |
| `auth=bearer:<token>` | No | Sends the header `Authorization: Bearer <token>`. The log shows the token as `***`. |

The name of the file is the last part of the URL path, without the query.
If the URL path has no name, the file is `download`. An existing file with
the same name is replaced. The action fails if the server responds with a
status that is not 2xx.

```markdown
- actions:
  - download https://example.com/report.pdf output_dir=./output mime=application/pdf
```

## `websearch`

```text
websearch topic="<text>" [max_results=<n>] [fetch_content=<true|false>] [base_url=<url>]
```

Searches one topic with the
[Tavily search API](https://docs.tavily.com/documentation/api-reference/endpoint/search).
The output is the Tavily response JSON, unchanged. The action ignores its
input and asks no model.

| Parameter | Required | Description |
| --- | --- | --- |
| `topic="<text>"` | Yes | What to search for. |
| `max_results=<n>` | No | The number of results, from 1 to 20. The default is 5. |
| `fetch_content=<true\|false>` | No | If `true`, each result also has its page as markdown, in `raw_content`. The default is `false`. |
| `base_url=<url>` | No | The server. The default is `https://api.tavily.com`. |

Before you run the action, set your Tavily API key:

```sh
export TAVILY_API_KEY=<your key>
```

Atama reads the key only from `TAVILY_API_KEY`, and it never writes the key
in the log. Each search uses Tavily API credits.

A missing topic, an unknown option or a bad value stops the job before it
starts. With `--debug`, the action writes its options and the response to
stderr. The old name `agent:websearch` is an error.

```markdown
- actions:
  - websearch max_results=5 fetch_content=true topic="latest stable Rust release"
  - write_file ./search.json
```

## `tavily_search`

```text
tavily_search [options...] [query...]
```

Searches the web with the
[Tavily search API](https://docs.tavily.com/documentation/api-reference/endpoint/search).
The output is the Tavily response JSON, unchanged. Each word that is not an
option is a word of the query. Without query words, the input of the action
is the query.

| Parameter | Required | Description |
| --- | --- | --- |
| `[query...]` | No | The words of the query. The default is the input. |
| `max_results=<n>` | No | The most results. |
| `search_depth=<depth>` | No | `basic` or `advanced`. |
| `topic=<topic>` | No | `general`, `news` or `finance`. |
| `include_answer=<value>` | No | `true`, `false`, `basic` or `advanced`. |
| `include_raw_content=<value>` | No | `true`, `false`, `markdown` or `text`. |
| `base_url=<url>` | No | The server. The default is `https://api.tavily.com`. |

If you do not set an option, Tavily uses its default. The action needs
`TAVILY_API_KEY`, as [`websearch`](#websearch) does.

The action reads its options when it runs, not when the job starts. It
fails if an option has no value or is set twice, if `max_results` is not a
number, or if the query is empty. A word with an unknown key, as in
`lang=en`, is a word of the query.

```markdown
- actions:
  - tavily_search max_results=3 topic=news latest stable Rust release
  - bash jq -r .results[].url
```

## `agent:<type>`

```text
agent:<type>
  - model=<profile>
  - <key>=<value>
  - ...
```

Sends a task to a chat model and runs the tools that the model asks for. The
first reply without a tool call is the output. The type is `coder`,
`summarizer` or `autonomous`.

The model must support tool calls. It must be on a server with an OpenAI
compatible `/chat/completions` endpoint, such as OpenAI, Ollama, vLLM or LM
Studio. To set up the model, see "Model profiles" in [README.md](README.md).

The options of an agent are a list under the action, one `- <key>=<value>`
bullet on each line. Indent the bullets more than the bullet of the action.
An option on the line of the action, as in `agent:coder model=local-qwen`,
stops the job before it starts. An unknown option, or an item without `=`,
also stops the job.

A value can have spaces, and it needs no quotes. If the whole value is in
double quotes, Atama removes them (see
[Quotes in parameters](#quotes-in-parameters)). To write a long value on
more than one line, indent the next lines more than the bullet. Atama joins
the lines with one space:

```markdown
- actions:
  - agent:coder
    - model=local-qwen
    - max_steps=15
    - task=Write a Python program that prints the first 10 prime numbers.
      Save it to ./output/primes.py and run it with python3.
```

In a `foreach agent:<type>` action, the list goes under the `foreach` line
in the same way.

Options of every agent:

| Parameter | Required | Description |
| --- | --- | --- |
| `model=<profile>` | Yes | The name of a model profile in the `config` block. An unknown name stops the job before it starts. |
| `task="<text>"` | No | The instruction. The input of the action goes below it. The action fails if it has no task and no input. |
| `max_steps=<n>` | No | The most model replies in one run, a number above 0. The default is 10, and 50 for `autonomous`. |
| `token_budget=<n>` | No | The most tokens in one run, a number above 0. An `_` between two digits separates groups of digits, as in `100_000`. It replaces the `token_budget=` of the model profile. Without both, there is no limit. See "Token budget" in [README.md](README.md). |
| `max_context=<n>` | No | The most tokens of the conversation, a number above 0. Before a request that is over it, Atama asks the model for a summary of the work so far, and sends the request with the system prompt, the task and the summary only. The summary costs tokens of the budget. Atama estimates 4 characters for one token. Without this option, Atama never summarizes. |
| `thinking=<true\|false>` | No | `false` sends `"reasoning_effort": "none"`, which turns off the thinking of the model on Ollama. Other servers can reject it. `true` is the same as no option. |
| `output_schema=<name\|path>` | No | Makes the output a JSON value that matches a schema. See "Output schemas" in [README.md](README.md). |

With `--debug`, the agent writes its configuration, each request, each reply
and each tool call to stderr. See "Debug output" in [README.md](README.md).

These agents ignore `effort=` of a model profile. If a profile with
`effort=` is the `model=` or the `judge_model=` of one of these agents,
Atama writes a red warning to stderr, and the job runs.

### `agent:coder`

A `coder` agent has the `bash` tool. The tool runs a command in the
directory where you start Atama. Atama cuts a tool result after 20,000
characters. A `coder` agent fails on the options `expertise=` and
`max_output=`, and on the options of `autonomous`.

If you use `agent:coder`, the model can run any command with your
permissions. Use it only in a directory and with a model that you trust.

```markdown
- actions:
  - agent:coder
    - model=local-qwen
    - task=Fix the failing test.
```

### `agent:summarizer`

A `summarizer` agent summarizes its input and keeps the key facts, such as
numbers, dates and names. It has no tools. It fails if the input is empty.
The default task is "Summarize the input."

It takes two more options:

| Parameter | Required | Description |
| --- | --- | --- |
| `expertise="<text>"` | No | The field of the reader, as in `expertise="molecular biology"`. The default is `general`. |
| `max_output="<size>"` | No | The most that the summary can be. See the sizes below. Without it, the summary is as short as it can be. |

The size of `max_output=` is one of these:

- A percent of the input, from 1 to 100, as in `60%`.
- A number of sentences, as in `"3 sentences"` or `"1 sentence"`.
- A number of words, as in `"200 words"`.
- A number of paragraphs, as in `"2 paragraphs"`.

```markdown
- actions:
  - read_file ./notes.md
  - agent:summarizer
    - model=local-qwen
    - expertise=software engineering
    - max_output=3 sentences
```

### `agent:autonomous`

An `autonomous` agent works on its task in iterations until its success
checks pass. An iteration is one attempt at the whole task. It ends when
the agent replies without tool calls, which means "I am done". Then the
success checks run. If a check fails, the agent gets the reason, and the
next iteration starts.

There are two success checks. You must give one of them or both:

- `success_check=` is a bash command. Exit status 0 means that the task is done.
- `success=` is success criteria in natural language. A separate request to the model of the agent, the `judge,` decides whether the work meets them.

If you give both, the command runs first, and the judge runs only if the
command passes.

**A step is one request to the model and its one reply.** For example, the
agent reads a file, edits it, runs the tests and says that it is done. That
is 4 steps in iteration 1. `max_iterations=` limits how many times the agent
can say that it is done and try again. `max_steps=` limits the steps of the
whole run.

The run stops at the first of these events:

- The success checks pass. The action succeeds, and the output is the last reply.
- The run used all its iterations, steps, tokens or time. The action fails, and the error has the last error of the success checks.
- The guard blocked `max_violations` tool calls. The action fails.

The agent has the tools `bash`, `read_file`, `write_file` and `list_files`.
Each tool has a limit of 50 calls and 10 failed calls in one run. A call
over a limit fails, and the next request does not offer the tool.

It takes these options:

| Parameter | Required | Description |
| --- | --- | --- |
| `success_check="<command>"` | One of the two | A bash command. Exit status 0 means that the task is done. The agent gets the exit status and the last 5,000 characters of stdout and stderr of a failed check. |
| `success="<criteria>"` | One of the two | Success criteria in natural language, which the judge examines. The agent gets the reason of a failed check. |
| `max_iterations=<n>` | No | The most iterations, a number above 0. The default is 5. |
| `max_time=<duration>` | No | The most wall clock time of the action, as in `90s`, `20m` or `1h`. The default is `30m`. The time counts from the start of the action. Atama examines the time before each request and each tool call. A `bash` call or a success check that still runs at the limit stops, with all its child processes. |
| `guardrails="<text>"` | No | Rules that the agent must obey, in natural language. Atama puts them into the system prompt. |
| `toolGuard=<prompt\|judge>` | No | `prompt`, the default, only puts the guardrails into the system prompt. `judge` also sends each tool call to the judge before it runs. A call that breaks a guardrail does not run, and the agent gets the reason. `judge` needs `guardrails=`. |
| `max_violations=<n>` | No | The most tool calls that the guard blocks before the action fails. The default is 3. |
| `protect="<patterns>"` | No | Globs of the paths that the tools cannot write, separated by commas, as in `protect="src/**,Cargo.toml"`. See [Protected paths](#protected-paths). |
| `judge_model=<profile>` | No | The name of a model profile in the `config` block for the judge and the guard. Without it, they use the model of the agent. It needs `success=` or `toolGuard=judge`. |
| `budget=<name>` | No | The name of a budget block with the general limits and tools of the agent. See [Budget blocks](#budget-blocks). |

The judge and the guard use the model profile of `judge_model=`, else the
model profile of the agent. For example, a small local model can judge the
work of a large one. The API key of the judge profile comes from its own
`api_key=` variable. The tokens of the judge and the guard count in the
token budget of the action. With `toolGuard=judge`, each tool call costs one
more request to the model.

The guardrails and the guard are text that a model reads, so they cannot
promise that a rule holds. For a rule about files, use `protect=`, which
Atama enforces without a model.

With `--debug`, each header of a run with more than one iteration names
the iteration, as in `iteration 2 step 3`. The header of each request also
shows the used time, as in `41s of 30m`.

If you use `agent:autonomous`, the model can run any command with your
permissions, and nobody approves each command. Use it only in a directory
and with a model that you trust. `protect=` keeps the agent from writing
the paths that you name, but it does not limit the other commands.

```markdown
- actions:
  - agent:autonomous
    - model=local-qwen
    - task=Make all tests pass.
    - success_check=cargo test
    - max_time=20m
    - protect=tests/**
  - agent:autonomous
    - model=local-qwen
    - task=Write a README for this project.
    - success=The README explains how to install, configure and run the tool.
    - guardrails=Do not change source files.
    - toolGuard=judge
```

#### Steps of a run

A run of an `autonomous` agent has these steps. The names in code font are
the types and functions in `src/agent_loop_openai_compat/`.

1. The loop starts with the system prompt and the task. It makes the list of `FinalLoopCheck`s from the options: `SchemaCheck`, `CommandCheck`, then `SuccessJudge`.
2. Before each step, `RunBudget` makes sure that the run still has steps, tokens and time. With `max_context=`, the loop compacts a conversation that is too long.
3. The loop sends the conversation to the model. The model replies with tool calls or with a final text.
4. With `toolGuard=judge`, `judge::review` asks the `Judge` about each tool call. If the `Verdict` has `pass: false`, the call does not run, and it counts as a violation.
5. If the guard allows the call, the tool runs. With `protect=`, a write to a protected path fails. The result goes into the conversation, and the loop goes back to step 2.
6. A reply without tool calls goes through the `FinalLoopCheck`s in order. `SchemaCheck` and `CommandCheck` decide without a model.
7. `SuccessJudge` asks the `Judge` with the criteria, the evidence and the reply. The `Verdict` becomes `Done` if `pass` is true, else `Retry` with its `reason`.
8. If all checks give `Done`, the run ends. The output of the last check is the output of the action.
9. If a check gives `Retry`, the model gets its message, and the loop goes on. A failed `CommandCheck` or `SuccessJudge` also starts the next iteration.

The `Judge` sends a separate request to the model, and the `Verdict` is its
answer: `pass` and a `reason`. A `FinalLoopCheck` is a rule that the reply
at the end of an iteration must pass. Of the checks, only `SuccessJudge`
uses a `Verdict`.

#### Protected paths

`protect=` names the paths that the agent cannot write, delete or rename.
It also protects the paths below them. The agent also cannot delete or
rename a directory above a protected path, because that moves the path.
The agent can still read the protected paths.

Each pattern is a glob of a path, relative to the working directory or
absolute:

- `*` matches any text in one part of the path, also a name that starts with `.`.
- `?` matches one character in one part of the path.
- `**` as a whole part matches any number of parts, as in `src/**/*.rs`.
- `[a-z]` matches one character in the range, and `[!x]` one character that is not `x`. A class can have only letters, digits, `-`, `_` and `.`.

Atama enforces the patterns in each tool using **OS sandboxing**, without a model:

- `write_file` does not write a protected path. It follows symbolic links and `..`, so a link to a protected file is protected too.
- `bash` runs each command inside the macOS sandbox, `sandbox-exec`. The sandbox denies each write to a protected path, whatever the command is. For example, `rm -rf`, `mv`, a redirect and a script all fail.

A blocked call fails, and the agent gets the error. The system prompt
tells the agent the protected paths, so that it does not try. A blocked
call counts as a failed call of the tool, not as a violation of the guard.

If you use `protect=` and the agent has the `bash` tool, the action needs
macOS, because only macOS has `sandbox-exec`. On another system, the action
fails before the first request. To use `protect=` on another system, remove
`bash` from `tools:` of the budget block.

`protect=` has these limits:

- The success check runs without the sandbox, because you write the command, not the agent.
- A command can ask another program that already runs to write a file, such as a daemon or `osascript`. The sandbox does not stop that program.

```markdown
- actions:
  - agent:autonomous
    - model=local-qwen
    - task=Fix the bug in the parser.
    - success_check=cargo test
    - protect=tests/**,Cargo.toml,.env
```

#### Budget blocks

A budget block holds the general configuration of an `autonomous` agent,
for example its limits and its tools. Many actions can use the same block.
A budget block is a ```` ```budget:<name> ```` block before the `# <name>`
heading of the job. The action selects it with `budget=<name>`.

The details of one action are options of the action, not of the block.
These options are `task=`, `protect=` and `guardrails=`. A budget block with
`task:`, `protect:` or `guardrails:` stops the job with an error.

````markdown
```budget:standard
max_iterations: 5
max_time: 20m
token_budget: 200_000
toolGuard: judge
tools:
  - read_file
  - list_files
  - write_file max_calls=20
  - bash max_calls=40 max_failures=5
```

# Fix the tests

## Fix

- actions:
  - agent:autonomous
    - model=local-qwen
    - budget=standard
    - task=Make all tests pass.
    - success_check=cargo test --quiet
    - protect=tests/**,Cargo.toml
    - guardrails=Do not run git push or git reset.
````

Each line is `<key>: <value>`. A key with no value starts a list, and each
item of the list is a `- <item>` line after it. The keys are the options
`max_steps`, `token_budget`, `max_context`, `thinking`, `max_iterations`,
`max_time`, `success_check`, `success`, `toolGuard`, `max_violations`,
`judge_model`, and `tools`. A list of text, such as `success:`, gives the
agent one `- <item>` line for each item.

An option of the action replaces the same option of the block. The action
needs `model=` in its options.

`tools:` selects the tools of the agent from `bash`, `read_file`,
`write_file` and `list_files`. Each tool can have `max_calls=<n>`, the most
calls in one run (default 50), and `max_failures=<n>`, the most failed
calls (default 10). Without `tools:`, the agent gets all four tools with the
default limits.

Atama reads the block and the options of every agent action before the job
starts. An unknown key, an unknown tool, an invalid option, or an
`autonomous` action without `success=` or `success_check=` stops the job
with the line of the error.

## `agent:claude-agent`

```text
agent:claude-agent
  - model=<profile>
  - <key>=<value>
  - ...
```

Runs a Claude Managed Agent with the Anthropic Managed Agents API, not
with `/chat/completions`. Anthropic runs the agent loop. Atama creates the
agent and the environment once, and keeps their IDs in
`.atama/claude-lock.json`. Then it starts a session, sends the task and the
input, runs the local tools that the agent calls, and answers each native
tool call that waits for approval. The output is the last message of the
agent. See [PLAN-claude-agent.md](PLAN-claude-agent.md) for the options and
the rules between them.

`success_check=<command>` is a bash command that Atama runs on the local
machine at the end of each turn of the agent. Exit status 0 means that the
task is done. If the command fails, Atama sends its exit status and the
last 5,000 characters of its stdout and stderr to the agent as the next
message. The check cannot see the files of the cloud environment, so the
agent must do its work with `local_tools=`. With `output_schema=`, the
reply must match the schema before the check runs. `max_turns=` (default
3) limits the messages that Atama sends, the task included.

The run fails at `max_time=`, at `max_cost_usd=`, over `token_budget=`, or
if the session fails. Then Atama sends `user.interrupt` to the session when
it can. Atama does not delete the session, so that you can read it in the
Claude Console. With `--debug`, Atama writes the link of the session and
each event to stderr.

The options are a list under the action, as for `agent:<type>`. The model
profile needs `api_key=`, and cannot have `thinking=`. `effort=` of the
profile sets the effort of the model. A `foreach agent:claude-agent` action
has the list under the `foreach` line.

A ```` ```claude-env:<name> ```` block before the `# <name>` heading
defines a cloud environment, and `environment=<name>` selects it. Each line
is `<key>: <value>`:

````markdown
```claude-env:research
networking: limited
allowed_hosts: arxiv.org, *.docs.rs
```
````

Without `name=`, the agent gets the name `atama-<job>-<task>-<n>`, in lower
case, where `<n>` is the number of the action in its task.

### Remote tools and local tools

A `claude-agent` action can use two kinds of tools. Without a tool option,
the agent has no tools and only replies with text.

| | Remote tools | Local tools |
| --- | --- | --- |
| Option | `native_tools=` | `local_tools=` |
| Tools | `bash`, `read`, `write`, `edit`, `glob`, `grep`, `web_fetch`, `web_search`, or `all` | `bash`, `read_file`, `write_file`, `list_files` |
| Where they run | In the cloud container of the session | On this machine, in the working directory of Atama |
| Who runs them | Anthropic | Atama, when the agent calls `local_<name>` |
| Files | Stay in the container. This machine cannot read them. | Are on this machine. `success_check=` and the next actions can read them. |
| Network | The environment of the session (`environment=`) limits it. | The network of this machine. |
| Approval | `autonomy=` sets the permission policy of the API. | `autonomy=` and `on_ask=`, in Atama. |
| Protection | `allowed_domains=` and `blocked_domains=` for the web tools | `protect=` |

Commas separate the names of a list. An action can have both options.

#### Remote tools

The remote tools are the native tools of the Claude agent. Use them when
the work does not need the files of this machine, for example to search
the web, to read pages, or to run code in a clean container.

````markdown
```claude-env:web
networking: limited
allowed_hosts: *.python.org, docs.rs
```

- actions:
  - agent:claude-agent
    - model=claude
    - environment=web
    - native_tools=web_search, web_fetch
    - allowed_domains=python.org, docs.rs
    - autonomy=ask_if_risky
    - max_cost_usd=0.50
    - task=Find the release date of the newest Python version.
````

These rules apply to the remote tools:

1. `web_search` and `web_fetch` need an environment that can reach the
   web. The default environment, `atama-default`, has `limited` networking
   with no hosts. So name a ```` ```claude-env:<name> ```` block with
   `allowed_hosts:`, or with `networking: unrestricted`, in
   `environment=`. Otherwise the job stops before it starts.
2. `allowed_domains=` names the only hosts that the web tools can reach.
   `blocked_domains=` names the hosts that they cannot reach. Use one of
   the two lists, not both. Each one needs `web_search` or `web_fetch`.
   The API rejects a host that the crawler of Anthropic cannot reach.
3. `autonomy=` becomes the permission policy of the API. `no_ask` lets
   each call run. `ask_if_risky`, the default, lets a safe call run, denies
   a high-risk call, and asks you about an unclear call. `always_ask` asks
   you about each call. `on_ask=` sets who answers: `prompt` asks on the
   terminal, `deny` denies the call, and `judge` asks a judge model (see
   "A judge for the calls that wait" below).
4. The web tools can read many pages, and each page costs tokens. Set
   `max_cost_usd=` or `token_budget=`. If you only need search results,
   the `websearch` action in an earlier step costs less.
5. An action with `agent_id=` uses an agent that exists, so it cannot set
   `native_tools=`. The agent keeps its own tools.

#### Local tools

The local tools are the tools of Atama. Use them when the agent must read
or write the files of this machine, or when `success_check=` must see its
work.

```markdown
- actions:
  - agent:claude-agent
    - model=claude
    - local_tools=read_file, write_file
    - autonomy=no_ask
    - protect=./jobs, ./src
    - success_check=test -s ./output/report.json
    - task=Write a report of ./data/results.csv to ./output/report.json.
```

These rules apply to the local tools:

1. The agent sees each tool with the prefix `local_`, as in
   `local_write_file`. Atama runs the call and sends the result back.
2. With `ask_if_risky`, `read_file` and `list_files` run, and `bash` and
   `write_file` wait for approval. With no terminal, for example in CI,
   `on_ask=prompt` denies the call. So for a job that runs alone, use
   `autonomy=no_ask` with `protect=`, or `on_ask=judge`.
3. `protect=` names the paths that the local tools cannot write. It needs
   `local_tools=`, and it does not apply to the remote tools.
4. Each local tool can run 50 times at most, and can fail 10 times at
   most.

#### A judge for the calls that wait

`on_ask=judge` sends each tool call that waits for approval to a judge
model, instead of to you. It is the same judge as the guard of
`toolGuard=judge` of an [`agent:autonomous`](#agentautonomous) action. The
same code (`judge::review`) asks the same question, so a judge profile and
guardrails that work for an `autonomous` action also work here.

| Option | Required | Description |
| --- | --- | --- |
| `on_ask=judge` | No | A judge decides each call that waits. |
| `judge_model=<profile>` | With `on_ask=judge` | The model profile of the judge, in the `config` block. It is an OpenAI compatible profile, as the `judge_model=` of an `autonomous` action. The API key comes from its own `api_key=` variable. |
| `guardrails=<rules>` | With `on_ask=judge` | The rules in natural language that the judge applies, as the `guardrails=` of an `autonomous` action. |

To change an `autonomous` action with `toolGuard=judge` to a
`claude-agent` action, keep `judge_model=` and `guardrails=`, and replace
`toolGuard=judge` with `on_ask=judge`:

````markdown
```config
claude: model=claude-opus-5-5, base_url=https://api.anthropic.com/v1, api_key=CLAUDE_API_KEY
local: model=qwen3, base_url=http://localhost:11434/v1, thinking=false
```

- actions:
  - agent:claude-agent
    - model=claude
    - local_tools=bash, write_file
    - autonomy=ask_if_risky
    - on_ask=judge
    - judge_model=local
    - guardrails=Write files only in ./output. Never delete files.
    - task=Make a report of the logs in ./logs in ./output/report.md.
````

These rules apply to the judge:

1. The judge gets the guardrails, the name of the tool and its input. It
   allows a call that breaks none of the guardrails. Else it denies the
   call, and the agent gets "blocked by guardrail: <reason>". A judge that
   fails denies the call.
2. The judge decides only the calls that wait. With
   `autonomy=ask_if_risky`, a remote call that the API finds safe and a
   local `read_file` or `list_files` run without the judge. With
   `always_ask`, the judge decides each call. `autonomy=no_ask` with
   `on_ask=judge` stops the job, because no call waits.
3. The tokens of the judge count in `token_budget=` of the action, with
   the tokens of the session. `max_cost_usd=` counts only the session.
4. `judge_model=` and `guardrails=` need `on_ask=judge`. To give the agent
   rules without a judge, put them in `system=`.
5. A denied call does not stop the run. The agent can try another way.
   Unlike `max_violations=` of an `autonomous` action, no limit counts the
   denied calls. `max_time=`, `token_budget=` and `max_cost_usd=` still
   stop the run.
6. With `--debug`, Atama writes each answer and the reason of each denied
   call to stderr.

#### Both kinds

An action can use both kinds, for example `web_fetch` to read a page in
the cloud and `local_write_file` to save the result on this machine. The
agent decides which tool to call, so name the tool in the task:

```markdown
- actions:
  - agent:claude-agent
    - model=claude
    - environment=web
    - native_tools=web_fetch
    - local_tools=write_file
    - autonomy=no_ask
    - protect=./jobs
    - task=Read https://docs.python.org/3/whatsnew/ with web_fetch.
      Then save a list of the new features to ./output/python.md with
      local_write_file.
```

A file that a remote tool writes stays in the container. To get it on
this machine, the agent must write it again with a local tool.

## `read_file`

```text
read_file [path]
```

Reads a text file. The output is the content of the file, with every line
unchanged.

| Parameter | Required | Description |
| --- | --- | --- |
| `[path]` | No | The path of the file. Put a path with spaces in quotes, as in `read_file "my notes.txt"`. The default is the input. |

Without a path, the input, without spaces at the start and the end, is the
path. The input must be one line. To read one file for each line, use
`foreach read_file`. The action fails if the file is not UTF-8 text.

```markdown
- actions:
  - read_file ./notes.txt
```

## `list_files`

```text
list_files [dir=<path>] [pattern=<glob>]
```

Lists the files of one directory whose names match a pattern. The output
has one path per line, sorted by name. Each line ends with a newline.

| Parameter | Required | Description |
| --- | --- | --- |
| `dir=<path>` | No | The directory. Put a path with spaces in quotes, as in `dir="my docs"`. The default is the input. |
| `pattern=<glob>` | No | The pattern of the file names. The default is `*`. |

Without `dir=`, the input, without spaces at the start and the end, is the
directory. The input must be one line. To list one directory for each line,
use `foreach list_files`.

The action does not go into subdirectories, and it does not list them. A
symbolic link to a file is in the list. Each path is the directory joined
with the file name, so that `foreach read_file` can read the files. If no
file matches, the output is empty, and the action succeeds.

A pattern (a glob) can have these special characters:

| Pattern | Matches |
| --- | --- |
| `*` | Any text, also no text. |
| `?` | One character. |
| `[abc]`, `[a-z]` | One character in the class or in the range. |
| `[!abc]`, `[^abc]` | One character that is not in the class. |
| `\*` | The character after `\`, as a plain character. |

As in a shell, a pattern matches a name that starts with `.` only if the
pattern also starts with `.`. For example, `*` does not match `.env`, but
`.*` does. A pattern with `/` is an error.

```markdown
- actions:
  - list_files dir=./notes pattern=*.md
  - foreach read_file
```

## `write_file`

```text
write_file <path>
```

Writes its input to a file, unchanged, and passes the input on as its
output, as `tee` does.

| Parameter | Required | Description |
| --- | --- | --- |
| `<path>` | Yes | The path of the file. Put a path with spaces in quotes. |

The action makes the missing parent directories, and it replaces an
existing file. The `write_file(...)` hook is a different thing, because it
runs after a task.

```markdown
- actions:
  - bash date
  - write_file ./output/result.txt
```

## `foreach`

```text
foreach <action>
```

Runs an action once for each line of its input, with that line as the
input. The output is the outputs of all runs, in order. Each output ends
with a newline.

| Parameter | Required | Description |
| --- | --- | --- |
| `<action>` | Yes | Any action line except another `foreach`. |

Atama skips blank lines. If one run fails, `foreach` stops, and the error
names the item, as in "`foreach` failed at item 2 (`banana`): ...".

```markdown
## List items

- actions:
  - bash printf %s\n apple banana cherry

## Describe each item

- actions:
  - foreach bash awk {print"[item:",$0"]","has",length($0),"characters"}
```

## `mock`

```text
mock
```

Does nothing. The output is the input, unchanged. The action has no
parameters. Use it as a placeholder for an action that you add later.

## `mock fail`

```text
mock fail
```

Always fails with the error "mock failure". The action has no other
parameters. Use it to test the `if_fail` hooks and the failure triggers of
a job.
