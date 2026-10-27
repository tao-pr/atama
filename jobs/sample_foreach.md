# Foreach sample

Makes a list of fruits, runs one action for each fruit, and writes all the
outputs to one text file. This file is a sample of the `foreach` action
prefix. It needs no model and no API key. Every file goes into
`./output/foreach/`.

To run the job:

```bash
cargo run -- jobs/sample_foreach.md
```

## Prepare

Delete the output of an earlier run, and make the output directory again.

- actions:
  - bash rm -rf ./output/foreach
  - bash mkdir -p ./output/foreach

## List items

Print one fruit on each line. `printf` repeats its format for each argument,
so the output has three lines. This list is the input of the next task.

- actions:
  - bash printf %s\n apple banana cherry

## Describe each item

`foreach` runs the action after it once for each line of its input. Each
run gets one line as its input, so `awk` sees only one fruit. The awk
program writes the fruit in a `[item: ...]` tag, and its length. Atama
joins the outputs of the three runs, in order, so the tag at the start of
each line shows which input item made it. The awk program has no spaces,
because Atama splits an action line at its spaces.

- actions:
  - foreach bash awk {print"[item:",$0"]","has",length($0),"characters"}

## Save results

Write the joined outputs to a text file. `tee` also passes them on as the
output of the task, so the log shows them.

- actions:
  - bash tee ./output/foreach/concat.txt
