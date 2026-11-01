# Sync items

Gets the items from a local API, saves them, sends them back with a bearer
token, downloads a sample text file and lists the files that the job wrote.
This file is a sample that uses every action type that Atama supports: `bash`,
`api_call`, `download`, `mock` and `mock fail`. Every file goes into `./output/`.

The API calls need a server on `localhost:8080`. See "Mock Server" in the README.

Prerequisites: Start a mock server with `siga`

```bash
../siga/target/release/siga --api jobs/sample-api.yaml
```

## Prepare

Make the output directory.

- actions:
  - bash mkdir -p ./output
  - bash rm -f ./output/*

## Fetch items

Get the items as JSON. `write_file` writes the JSON to a file and also
passes it on as the output of the task.

- actions:
  - api_call GET http://localhost:8080/api/items
  - write_file ./output/items.json
- if_fail: retry=2

## Update items

Send the items from the previous task back to the API. The request body is
the input of the action, which is the JSON from Fetch items.

- actions:
  - api_call PUT http://localhost:8080/api/items?force-auth=put auth=bearer:my-valid-token
  - mock check the response
- if_fail: retry=1

## Download sample

Download a text file to `./output/sample1.txt`. The file name comes from the
last part of the URL.

- actions:
  - download https://samples-files.com/samples/documents/txt/sample1.txt output_dir=./output mime=text/plain
- if_fail: retry=2

## List output

Print all the files that the job wrote.

- actions:
  - bash ls -a ./output

## Alert

Runs only if Update items fails. `mock` stands in for an alert that Atama
cannot send yet. `mock fail` then makes this task fail too.

- begin_if: task(Update items) fail
- actions:
  - mock send an alert
  - mock fail

## Record fetch error

Runs only if Fetch items fails. Writes `fetch-api-error <timestamp>` to
`./output/error-api-call.txt`. The bash actor passes each word to the command
unchanged, so `>` and `$(date)` do not work in one action. `date` prints the
timestamp, `awk` adds `fetch-api-error` in front of it, and `write_file` writes
the line to the file.

- begin_if: task(Fetch items) fail
- actions:
  - bash date -u +%Y-%m-%dT%H:%M:%SZ
  - bash awk {print"fetch-api-error",$0}
  - write_file ./output/error-api-call.txt
