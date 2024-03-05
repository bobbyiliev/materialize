# Copyright Materialize, Inc. and contributors. All rights reserved.
#
# Use of this software is governed by the Business Source License
# included in the LICENSE file at the root of this repository.
#
# As of the Change Date specified in that file, in accordance with
# the Business Source License, use of this software will be governed
# by the Apache License, Version 2.0.
#
# ci_logged_errors_detect.py - Detect errors in log files during CI and find
# associated open GitHub issues in Materialize repository.

import argparse
import mmap
import os
import re
import sys
from typing import Any

import requests

from materialize import ci_util, ui
from materialize.buildkite import add_annotation

CI_RE = re.compile("ci-regexp: (.*)")
CI_APPLY_TO = re.compile("ci-apply-to: (.*)")

# Unexpected failures, report them
ERROR_RE = re.compile(
    rb"""
    ^ .*
    ( segfault\ at
    | trap\ invalid\ opcode
    | general\ protection
    | has\ overflowed\ its\ stack
    | internal\ error:
    | \*\ FATAL:
    | [Oo]ut\ [Oo]f\ [Mm]emory
    | cannot\ migrate\ from\ catalog
    | halting\ process: # Rust unwrap
    | fatal runtime error: # stack overflow
    | \[SQLsmith\] # Unknown errors are logged
    | \[SQLancer\] # Unknown errors are logged
    | environmentd:\ fatal: # startup failure
    | clusterd:\ fatal: # startup failure
    | error:\ Found\ argument\ '.*'\ which\ wasn't\ expected,\ or\ isn't\ valid\ in\ this\ context
    | environmentd .* unrecognized\ configuration\ parameter
    | cannot\ load\ unknown\ system\ parameter\ from\ catalog\ storage
    )
    .* $
    """,
    re.VERBOSE | re.MULTILINE,
)

# Panics are multiline and our log lines of multiple services are interleaved,
# making them complex to handle in regular expressions, thus handle them
# separately.
# Example 1: launchdarkly-materialized-1  | thread 'coordinator' panicked at [...]
# Example 2: [pod/environmentd-0/environmentd] thread 'coordinator' panicked at [...]
PANIC_IN_SERVICE_START_RE = re.compile(
    rb"^(\[)?(?P<service>[^ ]*)(\s*\||\]) thread '.*' panicked at "
)
# Example 1: launchdarkly-materialized-1  | global timestamp must always go up
# Example 2: [pod/environmentd-0/environmentd] Unknown collection identifier u2082
SERVICES_LOG_LINE_RE = re.compile(rb"^(\[)?(?P<service>[^ ]*)(\s*\||\]) (?P<msg>.*)$")

PANIC_WITHOUT_SERVICE_START_RE = re.compile(rb"^thread '.*' panicked at ")
PANIC_ENDED_RE = re.compile(rb"note: Some details are omitted")

# Expected failures, don't report them
IGNORE_RE = re.compile(
    rb"""
    # Expected in restart test
    ( restart-materialized-1\ \ \|\ thread\ 'coordinator'\ panicked\ at\ 'can't\ persist\ timestamp
    # Expected in restart test
    | restart-materialized-1\ *|\ thread\ 'coordinator'\ panicked\ at\ 'external\ operation\ .*\ failed\ unrecoverably.*
    # Expected in cluster test
    | cluster-clusterd[12]-1\ .*\ halting\ process:\ new\ timely\ configuration\ does\ not\ match\ existing\ timely\ configuration
    # Emitted by tests employing explicit mz_panic()
    | forced\ panic
    # Emitted by broken_statements.slt in order to stop panic propagation, as 'forced panic' will unwantedly panic the `environmentd` thread.
    | forced\ optimizer\ panic
    # Expected once compute cluster has panicked, brings no new information
    | timely\ communication\ error:
    # Expected once compute cluster has panicked, only happens in CI
    | aborting\ because\ propagate_crashes\ is\ enabled
    # Expected when CRDB is corrupted
    | restart-materialized-1\ .*relation\ \\"fence\\"\ does\ not\ exist
    # Expected when CRDB is corrupted
    | restart-materialized-1\ .*relation\ "consensus"\ does\ not\ exist
    # Will print a separate panic line which will be handled and contains the relevant information (new style)
    | internal\ error:\ unexpected\ panic\ during\ query\ optimization
    # redpanda INFO logging
    | larger\ sizes\ prevent\ running\ out\ of\ memory
    # Old versions won't support new parameters
    | (platform-checks|legacy-upgrade|upgrade-matrix|feature-benchmark)-materialized-.* \| .*cannot\ load\ unknown\ system\ parameter\ from\ catalog\ storage
    # Fencing warnings are OK in fencing tests
    | persist-txn-fencing-mz_first-.* \| .*unexpected\ fence\ epoch
    | persist-txn-fencing-mz_first-.* \| .*fenced\ by\ new\ catalog\ upper
    | platform-checks-mz_txn_tables.* \| .*unexpected\ fence\ epoch
    | platform-checks-mz_txn_tables.* \| .*fenced\ by\ new\ catalog\ upper
    # For platform-checks upgrade tests
    | platform-checks-clusterd.* \| .* received\ persist\ state\ from\ the\ future
    | cannot\ load\ unknown\ system\ parameter\ from\ catalog\ storage(\ to\ set\ (default|configured)\ parameter)?
    | internal\ error:\ no\ AWS\ external\ ID\ prefix\ configured
    # For persist-catalog-migration ignore failpoint panics
    | persist-catalog-migration-materialized.* \| .* failpoint\ .* panic
    # For tests we purposely trigger this error
    | persist-catalog-migration-materialized.* \| .* incompatible\ persist\ version\ \d+\.\d+\.\d+(-dev)?,\ current:\ \d+\.\d+\.\d+(-dev)?,\ make\ sure\ to\ upgrade\ the\ catalog\ one\ version\ at\ a\ time
    )
    """,
    re.VERBOSE | re.MULTILINE,
)


class KnownIssue:
    def __init__(self, regex: re.Pattern[Any], apply_to: str | None, info: Any):
        self.regex = regex
        self.apply_to = apply_to
        self.info = info


class ErrorLog:
    def __init__(self, match: bytes, file: str):
        self.match = match
        self.file = file


def main() -> int:
    parser = argparse.ArgumentParser(
        prog="ci-logged-errors-detect",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        description="""
ci-logged-errors-detect detects errors in log files during CI and finds
associated open GitHub issues in Materialize repository.""",
    )

    parser.add_argument("log_files", nargs="+", help="log files to search in")
    args = parser.parse_args()

    return annotate_logged_errors(args.log_files)


def annotate_errors(errors: list[str], title: str, style: str) -> None:
    if not errors:
        return

    errors = group_identical_errors(errors)
    suite_name = os.getenv("BUILDKITE_LABEL") or "Logged Errors"

    error_str = "\n".join(f"* {error}" for error in errors)
    add_annotation(style=style, title=f"{suite_name}: {title}", content=error_str)


def group_identical_errors(errors: list[str]) -> list[str]:
    errors_with_counts: dict[str, int] = {}

    for error in errors:
        errors_with_counts[error] = 1 + errors_with_counts.get(error, 0)

    consolidated_errors = []

    for error, count in errors_with_counts.items():
        consolidated_errors.append(
            error if count == 1 else f"{error}\n({count} occurrences)"
        )

    return consolidated_errors


def annotate_logged_errors(log_files: list[str]) -> int:
    """
    Returns the number of unknown errors, 0 when all errors are known or there
    were no errors logged. This will be used to fail a test even if the test
    itself succeeded, as long as it had any unknown error logs.
    """

    error_logs = get_error_logs(log_files)

    if not error_logs:
        return 0

    step_key: str = os.getenv("BUILDKITE_STEP_KEY", "")
    buildkite_label: str = os.getenv("BUILDKITE_LABEL", "")

    (known_issues, unknown_errors) = get_known_issues_from_github()

    artifacts = ci_util.get_artifacts()
    job = os.getenv("BUILDKITE_JOB_ID")

    known_errors: list[str] = []

    # Keep track of known errors so we log each only once
    already_reported_issue_numbers: set[int] = set()

    for error in error_logs:
        error_message = sanitize_text(error.match.decode("utf-8"))
        formatted_error_message = f"```\n{error_message}\n```"

        for artifact in artifacts:
            if artifact["job_id"] == job and artifact["path"] == error.file:
                org = os.environ["BUILDKITE_ORGANIZATION_SLUG"]
                pipeline = os.environ["BUILDKITE_PIPELINE_SLUG"]
                build = os.environ["BUILDKITE_BUILD_NUMBER"]
                linked_file = f'[{error.file}](https://buildkite.com/organizations/{org}/pipelines/{pipeline}/builds/{build}/jobs/{artifact["job_id"]}/artifacts/{artifact["id"]})'
                break
        else:
            linked_file = error.file

        for issue in known_issues:
            match = issue.regex.search(error.match)
            if match and issue.info["state"] == "open":
                if issue.apply_to and issue.apply_to not in (
                    step_key.lower(),
                    buildkite_label.lower(),
                ):
                    continue

                if issue.info["number"] not in already_reported_issue_numbers:
                    known_errors.append(
                        f"[{issue.info['title']} (#{issue.info['number']})]({issue.info['html_url']}) in {linked_file}:  \n{formatted_error_message}"
                    )
                    already_reported_issue_numbers.add(issue.info["number"])
                break
        else:
            for issue in known_issues:
                match = issue.regex.search(error.match)
                if match and issue.info["state"] == "closed":
                    if issue.apply_to and issue.apply_to not in (
                        step_key.lower(),
                        buildkite_label.lower(),
                    ):
                        continue

                    if issue.info["number"] not in already_reported_issue_numbers:
                        unknown_errors.append(
                            f"Potential regression [{issue.info['title']} (#{issue.info['number']}, closed)]({issue.info['html_url']}) in {linked_file}:  \n{formatted_error_message}"
                        )
                        already_reported_issue_numbers.add(issue.info["number"])
                    break
            else:
                unknown_errors.append(
                    f"Unknown error in {linked_file}:  \n{formatted_error_message}"
                )

    annotate_errors(
        unknown_errors,
        "Unknown errors and regressions in logs (see [ci-regexp](https://github.com/MaterializeInc/materialize/blob/main/doc/developer/ci-regexp.md))",
        "error",
    )
    annotate_errors(known_errors, "Known errors in logs, ignoring", "info")

    if unknown_errors:
        print(
            f"+++ Failing test because of {len(unknown_errors)} unknown error(s) in logs:"
        )
        print(unknown_errors)

    return len(unknown_errors)


def get_error_logs(log_file_names: list[str]) -> list[ErrorLog]:
    error_logs = []
    for log_file_name in log_file_names:
        with open(log_file_name, "r+") as f:
            try:
                data: Any = mmap.mmap(f.fileno(), 0)
            except ValueError:
                # empty file, ignore
                continue

            error_logs.extend(_collect_errors_in_logs(data, log_file_name))
            error_logs.extend(_collect_service_panics_in_logs(data, log_file_name))
            error_logs.extend(_collect_loose_panics_in_logs(data, log_file_name))

    return error_logs


def _collect_errors_in_logs(data: Any, log_file_name: str) -> list[ErrorLog]:
    collected_errors = []

    for match in ERROR_RE.finditer(data):
        if IGNORE_RE.search(match.group(0)):
            continue
        # environmentd segfaults during normal shutdown in coverage builds, see #20016
        # Ignoring this in regular ways would still be quite spammy.
        if (
            b"environmentd" in match.group(0)
            and b"segfault at" in match.group(0)
            and ui.env_is_truthy("CI_COVERAGE_ENABLED")
        ):
            continue
        collected_errors.append(ErrorLog(match.group(0), log_file_name))

    return collected_errors


def _collect_service_panics_in_logs(data: Any, log_file_name: str) -> list[ErrorLog]:
    collected_panics = []

    open_panics = {}
    for line in iter(data.readline, b""):
        line = line.rstrip(b"\n")
        if match := PANIC_IN_SERVICE_START_RE.match(line):
            service = match.group("service")
            assert (
                service not in open_panics
            ), f"Two panics of same service {service} interleaving: {line}"
            open_panics[service] = line
        elif open_panics:
            if match := SERVICES_LOG_LINE_RE.match(line):
                # Handling every services.log line here, filter to
                # handle only the ones which are currently in a panic
                # handler:
                if panic_start := open_panics.get(match.group("service")):
                    del open_panics[match.group("service")]
                    if IGNORE_RE.search(match.group(0)):
                        continue
                    collected_panics.append(
                        ErrorLog(panic_start + b" " + match.group("msg"), log_file_name)
                    )
    assert not open_panics, f"Panic log never finished: {open_panics}"

    return collected_panics


def _collect_loose_panics_in_logs(data: Any, log_file_name: str) -> list[ErrorLog]:
    collected_panics = []

    open_panic: str | None = None
    end_of_panic_reached = True

    for line in iter(data.readline, b""):
        line = line.rstrip(b"\n")
        if PANIC_WITHOUT_SERVICE_START_RE.match(line):
            assert open_panic is None, "A panic is already pending"
            open_panic = line
            end_of_panic_reached = False
        elif open_panic is not None:
            if end_of_panic_reached:
                collected_panics.append(
                    ErrorLog(open_panic + " " + line, log_file_name)
                )
                open_panic = None
            else:
                if PANIC_ENDED_RE.search(line):
                    end_of_panic_reached = True

    assert open_panic is None, f"Panic log never finished: {open_panic}"

    return collected_panics


def sanitize_text(text: str, max_length: int = 4000) -> str:
    if len(text) > max_length:
        text = text[:max_length] + " [...]"

    text = text.replace("```", r"\`\`\`")

    return text


def get_known_issues_from_github_page(page: int = 1) -> Any:
    headers = {
        "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
    }
    if token := os.getenv("GITHUB_TOKEN"):
        headers["Authorization"] = f"Bearer {token}"

    response = requests.get(
        f'https://api.github.com/search/issues?q=repo:MaterializeInc/materialize%20type:issue%20in:body%20"ci-regexp%3A"&per_page=100&page={page}',
        headers=headers,
    )

    if response.status_code != 200:
        raise ValueError(f"Bad return code from GitHub: {response.status_code}")

    issues_json = response.json()
    assert issues_json["incomplete_results"] == False
    return issues_json


def get_known_issues_from_github() -> tuple[list[KnownIssue], list[str]]:
    page = 1
    issues_json = get_known_issues_from_github_page(page)
    while issues_json["total_count"] > len(issues_json["items"]):
        page += 1
        next_page_json = get_known_issues_from_github_page(page)
        if not next_page_json["items"]:
            break
        issues_json["items"].extend(next_page_json["items"])

    unknown_errors = []
    known_issues = []

    for issue in issues_json["items"]:
        matches = CI_RE.findall(issue["body"])
        matches_apply_to = CI_APPLY_TO.findall(issue["body"])
        for match in matches:
            try:
                regex_pattern = re.compile(match.strip().encode("utf-8"))
            except:
                unknown_errors.append(
                    f"[{issue.info['title']} (#{issue.info['number']})]({issue.info['html_url']}): Invalid regex in ci-regexp: {match.strip()}, ignoring"
                )
                continue

            if matches_apply_to:
                for match_apply_to in matches_apply_to:
                    known_issues.append(
                        KnownIssue(regex_pattern, match_apply_to.strip().lower(), issue)
                    )
            else:
                known_issues.append(KnownIssue(regex_pattern, None, issue))

    return (known_issues, unknown_errors)


if __name__ == "__main__":
    sys.exit(main())
