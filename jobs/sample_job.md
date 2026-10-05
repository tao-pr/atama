# Website deploy

Builds the static website, checks it, deploys it to staging and then production,
and reports the result. This file is a sample that uses every trigger condition
and hook Atama supports.

## Install

Install the build dependencies. Network hiccups are common, so retry a few times.

- actions:
  - npm ci
- if_fail: retry=3

## Build

Build the site into `dist/`. Runs after Install succeeds (the default).

- actions:
  - npm run build
- if_fail: write_error_file(logs/build-errors.log)
- if_succeed: write_file(logs/build.log)

## Lint

Runs at the same time as Build, so both start as soon as Install succeeds.

- begin_if: task(Build) begin
- actions:
  - npm run lint
- if_fail: write_error_file(logs/lint-errors.log)

## Test

- begin_if: task(Build) succeed
- actions:
  - npm test
- if_fail: die

## Deploy staging

- begin_if: prev task succeed
- actions:
  - ./scripts/deploy.sh staging
- if_fail: retry=2

## Smoke test

If staging looks broken, start the whole job over from Install.

- begin_if: task(Deploy staging) succeed
- actions:
  - ./scripts/smoke_test.sh https://staging.example.com
- if_fail: restart

## Deploy production

- begin_if: task(Smoke test) succeed
- actions:
  - ./scripts/deploy.sh production
- if_succeed: call_api(https://hooks.example.com/deploy-succeeded)

## Rollback

Only runs if the production deploy fails.

- begin_if: prev task fail
- actions:
  - ./scripts/rollback.sh production
- if_succeed: call_api(https://hooks.example.com/deploy-rolled-back)

## Report

Always runs once the production deploy has finished, whatever its result.

- begin_if: task(Deploy production) succeed or fail
- actions:
  - ./scripts/report.sh

## Clean up

Remove temporary build files after the report, whether or not it succeeded.

- begin_if: prev task succeed or fail
- actions:
  - rm -rf tmp/
