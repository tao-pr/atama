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
| [`agent:<type>`](#agenttype) | Runs a `coder` or `summarizer` agent. | The final reply of the model |
| [`read_file`](#read_file) | Reads a text file. | The content of the file, unchanged |
| [`write_file`](#write_file) | Writes the input to a file. | The input, unchanged |
| [`foreach`](#foreach) | Runs an action once for each input line. | All outputs, one after the other |
| [`mock`](#mock) | Does nothing. | The input, unchanged |
| [`mock fail`](#mock-fail) | Always fails. | None |

## Quotes in parameters

Some actions accept a value in double quotes, so that the value can have
spaces, as in `task="Fix the failing test."`. Inside quotes, `\"` is a quote
and `\\` is a backslash. Atama removes the quotes before it uses the value.

These actions accept quotes: `websearch`, `agent:<type>`, `read_file` and
`write_file`. The other actions split the line at each space, and they keep
quote characters as they are.

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
agent:<type> model=<profile> [options...]
```

Sends a task to a chat model and runs the tools that the model asks for. The
first reply without a tool call is the output. The type is `coder` or
`summarizer`.

The model must support tool calls. It must be on a server with an OpenAI
compatible `/chat/completions` endpoint, such as OpenAI, Ollama, vLLM or LM
Studio. To set up the model, see "Model profiles" in [README.md](README.md).

Each word after `agent:<type>` is a `key=value` option. An unknown option, or
a word without `=`, makes the action fail. Put a value with spaces in double
quotes (see [Quotes in parameters](#quotes-in-parameters)).

Options of every agent:

| Parameter | Required | Description |
| --- | --- | --- |
| `model=<profile>` | Yes | The name of a model profile in the `config` block. An unknown name stops the job before it starts. |
| `task="<text>"` | No | The instruction. The input of the action goes below it. The action fails if it has no task and no input. |
| `max_steps=<n>` | No | The most model replies in one run, a number above 0. The default is 10. |
| `thinking=<true\|false>` | No | `false` sends `"reasoning_effort": "none"`, which turns off the thinking of the model on Ollama. Other servers can reject it. `true` is the same as no option. |
| `output_schema=<name\|path>` | No | Makes the output a JSON value that matches a schema. See "Output schemas" in [README.md](README.md). |

With `--debug`, the agent writes its configuration, each request, each reply
and each tool call to stderr. See "Debug output" in [README.md](README.md).

### `agent:coder`

A `coder` agent has the `bash` tool. The tool runs a command in the
directory where you start Atama. Atama cuts a tool result after 20,000
characters. A `coder` agent fails on the options `expertise=` and
`max_output=`.

If you use `agent:coder`, the model can run any command with your
permissions. Use it only in a directory and with a model that you trust.

```markdown
- actions:
  - agent:coder model=local-qwen task="Fix the failing test."
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
  - agent:summarizer model=local-qwen expertise="software engineering" max_output="3 sentences"
```

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
