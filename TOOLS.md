# Tools

This file lists the tools that the model of an agent can call, and tells
which agents can use each tool. For the actions of a job file, see
[ACTIONS.md](ACTIONS.md). For the format of a job file, see
[README.md](README.md).

A tool is different from an action. An action is one step of a task, and
Atama runs it. A tool is a function that the model of an agent calls
during its run. The model, not the job file, decides when to call a tool
and with which arguments.

There are two kinds of tools:

- A local tool is built into Atama. Atama runs it on this machine, in the
  working directory of Atama. Its files stay on this machine, so
  `success_check=` and the next actions can read them.
- A provider tool is built into the API of the model provider. The
  provider runs it on its own servers. Atama does not see its files.

## All tools

| Tool | Kind | Where it runs | Agents that can use it | Read-only |
| --- | --- | --- | --- | --- |
| [`bash`](#bash) | Local | This machine | `coder`, `autonomous`, `claude-agent` | No |
| [`read_file`](#read_file) | Local | This machine | `autonomous`, `claude-agent` | Yes |
| [`write_file`](#write_file) | Local | This machine | `autonomous`, `claude-agent` | No |
| [`list_files`](#list_files) | Local | This machine | `autonomous`, `claude-agent` | Yes |
| [`web_search`](#web_search) | Local | This machine, which sends the search to Tavily | `autonomous`, `claude-agent` | Yes |
| [`bash`](#claude-bash) | Provider (Claude) | The cloud container of the session | `claude-agent` | No |
| [`read`](#claude-read) | Provider (Claude) | The cloud container of the session | `claude-agent` | Yes |
| [`write`](#claude-write) | Provider (Claude) | The cloud container of the session | `claude-agent` | No |
| [`edit`](#claude-edit) | Provider (Claude) | The cloud container of the session | `claude-agent` | No |
| [`glob`](#claude-glob) | Provider (Claude) | The cloud container of the session | `claude-agent` | Yes |
| [`grep`](#claude-grep) | Provider (Claude) | The cloud container of the session | `claude-agent` | Yes |
| [`web_fetch`](#claude-web_fetch) | Provider (Claude) | The servers of Anthropic | `claude-agent` | Yes |
| [`web_search`](#claude-web_search) | Provider (Claude) | The servers of Anthropic | `claude-agent` | Yes |

A read-only tool does not change files. With `autonomy=ask_if_risky`, a
read-only local tool runs without approval (see
[Approval](#approval-of-local-tools)).

The OpenAI compatible agents (`coder`, `summarizer` and `autonomous`) have
no provider tools. Atama sends only its local tools in the `tools` field of
the `/chat/completions` request (see
[OpenAI compatible servers](#openai-compatible-servers)).

## Tools of each agent

| Agent | Tools without options | How to change the tools |
| --- | --- | --- |
| [`agent:coder`](ACTIONS.md#agentcoder) | `bash` | You cannot change them. |
| [`agent:summarizer`](ACTIONS.md#agentsummarizer) | None. The model can only reply with text. | You cannot change them. |
| [`agent:autonomous`](ACTIONS.md#agentautonomous) | `bash`, `read_file`, `write_file` and `list_files` | `tools:` of a [budget block](ACTIONS.md#budget-blocks). Only `tools:` gives `web_search`. |
| [`agent:claude-agent`](ACTIONS.md#agentclaude-agent) | None. The model can only reply with text. | `local_tools=` for the local tools, and `native_tools=` for the provider tools |

## Local tools

The local tools are in `src/tools/`. They run the same code as the actions
of the same name, so `read_file` as a tool and as an action read a file in
the same way.

These rules apply to every local tool:

1. If a result has more than 20,000 characters (`MAX_RESULT_CHARS`), Atama
   cuts it and adds a line that says so.
2. A failed call does not stop the run. The model gets the error text as
   the result, and it can try again.
3. A tool of an `autonomous` or `claude-agent` run can run 50 times at
   most, and it can fail 10 times at most (`Limited`). After a limit, the
   model no longer sees the tool. The `tools:` list of a budget block can
   change the limits with `max_calls=<n>` and `max_failures=<n>`.
4. A `claude-agent` sees each local tool with the prefix `local_`, as in
   `local_write_file`. The prefix keeps the name apart from the provider
   tools, and it tells the model where the tool runs.

### `bash`

Runs a command in bash in the working directory, and returns its stdout.
It is the same as the [`bash`](ACTIONS.md#bash) action.

| Argument | Required | Description |
| --- | --- | --- |
| `command` | Yes | The command to run. |

If the command exits with a status that is not zero, the call fails with
its stderr. The command runs with your permissions, so the model can run
any command that you can run.

With `max_time=`, the command stops at the deadline of the run. Atama kills
the command and its child processes. With `protect=`, the command runs
inside `sandbox-exec`, which denies the writes to the protected paths (see
[Protected paths](ACTIONS.md#protected-paths)). Only macOS has
`sandbox-exec`.

### `read_file`

Returns the content of a text file. It is the same as the
[`read_file`](ACTIONS.md#read_file) action.

| Argument | Required | Description |
| --- | --- | --- |
| `path` | Yes | The path of the file, relative to the working directory. |

### `write_file`

Writes text to a file, and makes the missing parent directories. The file
gets only the new text. It is the same as the
[`write_file`](ACTIONS.md#write_file) action.

| Argument | Required | Description |
| --- | --- | --- |
| `path` | Yes | The path of the file, relative to the working directory. |
| `content` | Yes | The new content of the file. |

The result is a short confirmation, as in ``wrote 120 characters to
`out.txt` ``, not the content. With `protect=`, the tool does not write a
protected path. It resolves the real path first, so a symbolic link or a
`..` does not get past the check.

### `list_files`

Lists the files of one directory whose names match a glob pattern, one
path on each line, sorted. It does not go into subdirectories. It is the
same as the [`list_files`](ACTIONS.md#list_files) action.

| Argument | Required | Description |
| --- | --- | --- |
| `dir` | Yes | The directory. |
| `pattern` | No | The glob pattern of the file names, as in `*.rs`. The default is `*`. |

### `web_search`

Searches the web with Tavily, and returns the response JSON unchanged. It
runs the same code as the [`websearch`](ACTIONS.md#websearch) action, but
it does not get the content of the result pages, so that the result stays
short.

| Argument | Required | Description |
| --- | --- | --- |
| `query` | Yes | What to search for. |
| `max_results` | No | The most results, from 1 to 20. The default is 5. |

The tool needs the Tavily API key in the `TAVILY_API_KEY` environment
variable. If it is not set, the call fails, and the model gets the error.

No agent gets this tool by default, because each search uses Tavily
credits. To give it to an agent, do one of these steps:

- For `agent:autonomous`, add `- web_search` to `tools:` of the budget
  block.
- For `agent:claude-agent`, add `web_search` to `local_tools=`. The agent
  sees the tool as `local_web_search`, so it does not clash with the
  [Claude `web_search`](#claude-web_search).

A search with this tool costs less than the Claude `web_search` and
`web_fetch` tools, because the model reads only the short extracts.

```markdown
- actions:
  - agent:claude-agent
    - model=claude
    - local_tools=web_search
    - autonomy=no_ask
    - task=Find the release date of the newest Python version.
```

### Approval of local tools

A `claude-agent`, `coder` or `autonomous` action can make a local tool wait
for approval before it runs. `autonomy=` sets which calls wait, and
`on_ask=` sets who answers.

| `autonomy=` | Which calls wait |
| --- | --- |
| `no_ask` | No call. This is the default of `coder` and `autonomous`. |
| `ask_if_risky` | Each call of `bash` and `write_file`. The read-only tools `read_file`, `list_files` and `web_search` run. This is the default of `claude-agent`. |
| `always_ask` | Each call. |

`on_ask=prompt` asks you on the terminal, `on_ask=deny` denies the call, and
`on_ask=judge` asks a judge model. See
[Approval of the tool calls](ACTIONS.md#approval-of-the-tool-calls) for
`coder` and `autonomous`, and
[A judge for the calls that wait](ACTIONS.md#a-judge-for-the-calls-that-wait)
for `claude-agent`. An `autonomous` agent can also block a call with
`toolGuard=judge` (see [`agent:autonomous`](ACTIONS.md#agentautonomous)),
but not together with `autonomy=`.

## Provider tools

### OpenAI compatible servers

The `coder`, `summarizer` and `autonomous` agents send their requests to
an OpenAI compatible `/chat/completions` endpoint. The request has only
the local tools of the agent, each one as a `function` tool. So these
agents get no tool of the server, also when the server is OpenAI.

### Claude

A `claude-agent` action can turn on the native tools of a Claude Managed
Agent with `native_tools=`. The value is `all`, or a list of names that
commas separate:

```markdown
- actions:
  - agent:claude-agent
    - model=claude
    - environment=web
    - native_tools=web_search, web_fetch
    - allowed_domains=python.org, docs.rs
```

Without `native_tools=`, the agent has no native tools. These rules apply
to every native tool:

1. Anthropic runs the tool. The files of the tools are in the cloud
   container of the session. This machine cannot read them.
2. `autonomy=` becomes the permission policy of the API. `no_ask` sends
   `always_allow`, `ask_if_risky` sends `auto`, and `always_ask` sends
   `always_ask`.
3. An action with `agent_id=` uses an agent that exists, so it cannot set
   `native_tools=`.
4. The tool calls cost tokens. Set `max_cost_usd=` or `token_budget=`.

The details of the options are in
[Remote tools and local tools](ACTIONS.md#remote-tools-and-local-tools).

#### Claude `bash`

Runs a command in the cloud container of the session. The local tool of
the same name has the API name `local_bash`, so the two do not clash.

#### Claude `read`

Reads a file of the container.

#### Claude `write`

Writes a file of the container.

#### Claude `edit`

Replaces text in a file of the container.

#### Claude `glob`

Finds the files of the container whose paths match a glob pattern.

#### Claude `grep`

Searches the content of the files of the container.

#### Claude `web_fetch`

Gets the content of a web page. The environment of the session must reach
the web, so name a ```` ```claude-env:<name> ```` block with
`allowed_hosts:`, or with `networking: unrestricted`, in `environment=`.
`allowed_domains=` names the only hosts that the tool can reach, and
`blocked_domains=` names the hosts that it cannot reach. Use one of the two
lists, not both.

Each page costs tokens. A run of the sample job read whole pages and used
its budget of $0.50 in one step.

#### Claude `web_search`

Searches the web. The rules of the environment and of the domain lists are
the same as for `web_fetch`.

If you only need search results, the local [`web_search`](#web_search)
tool, or the [`websearch`](ACTIONS.md#websearch) action in an earlier
step, costs less.
