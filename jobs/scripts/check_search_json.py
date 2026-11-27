#!/usr/bin/env python3
"""Checks the search results of jobs/sample_approval.md.

Usage: check_search_json.py <trimmed|takeaways> <file>

trimmed: the file has only "query" and "results", and each result has
only "title", "url" and "content".
takeaways: each result also has a "takeaway" that is not empty.

Exits with status 1 and a message on stderr if the check fails.
"""

import json
import sys

FIELDS = {"title", "url", "content"}


def main():
    if len(sys.argv) != 3 or sys.argv[1] not in ("trimmed", "takeaways"):
        sys.exit("usage: check_search_json.py <trimmed|takeaways> <file>")
    mode, path = sys.argv[1], sys.argv[2]
    with open(path) as file:
        data = json.load(file)
    results = data.get("results")
    if not results:
        sys.exit(f"{path} has no results")
    if mode == "trimmed":
        if set(data) != {"query", "results"}:
            sys.exit(f"{path} has the fields {sorted(data)}, expected query and results")
        for index, result in enumerate(results):
            if set(result) != FIELDS:
                sys.exit(f"result {index} has the fields {sorted(result)}, expected {sorted(FIELDS)}")
    else:
        for index, result in enumerate(results):
            if not FIELDS <= set(result):
                sys.exit(f"result {index} lost a field, it has {sorted(result)}")
            if not str(result.get("takeaway", "")).strip():
                sys.exit(f"result {index} has no takeaway")
    print(f"{len(results)} results")


if __name__ == "__main__":
    main()
