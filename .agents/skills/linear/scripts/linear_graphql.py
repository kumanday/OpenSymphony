#!/usr/bin/env python3
"""Execute one Linear GraphQL operation using LINEAR_API_KEY."""

from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.request
from pathlib import Path


DEFAULT_ENDPOINT = "https://api.linear.app/graphql"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Run one Linear GraphQL operation using LINEAR_API_KEY."
    )
    parser.add_argument(
        "--query-file",
        required=True,
        help="Path to a .graphql file containing exactly one operation.",
    )
    parser.add_argument(
        "--variables",
        help="Inline JSON object for GraphQL variables.",
    )
    parser.add_argument(
        "--variables-file",
        help="Path to a JSON file containing GraphQL variables.",
    )
    parser.add_argument(
        "--endpoint",
        default=os.environ.get("LINEAR_GRAPHQL_ENDPOINT", DEFAULT_ENDPOINT),
        help="GraphQL endpoint. Defaults to Linear's public API.",
    )
    args = parser.parse_args()
    if args.variables and args.variables_file:
        parser.error("use either --variables or --variables-file, not both")
    return args


def load_query(path: str) -> str:
    return Path(path).read_text(encoding="utf-8")


def load_variables(args: argparse.Namespace) -> dict[str, object]:
    if args.variables_file:
        return json.loads(Path(args.variables_file).read_text(encoding="utf-8"))
    if args.variables:
        return json.loads(args.variables)
    return {}


def main() -> int:
    args = parse_args()
    api_key = os.environ.get("LINEAR_API_KEY", "").strip()
    if not api_key:
        print(
            json.dumps(
                {"error": "LINEAR_API_KEY is required to call Linear GraphQL."},
                indent=2,
            ),
            file=sys.stderr,
        )
        return 1

    payload = {
        "query": load_query(args.query_file),
        "variables": load_variables(args),
    }
    request = urllib.request.Request(
        args.endpoint,
        data=json.dumps(payload).encode("utf-8"),
        headers={
            "Content-Type": "application/json",
            "Authorization": api_key,
        },
        method="POST",
    )

    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            body = response.read().decode("utf-8")
    except urllib.error.HTTPError as error:
        body = error.read().decode("utf-8")
        print(body)
        return 1

    data = json.loads(body)
    print(json.dumps(data, indent=2, sort_keys=True))
    return 2 if data.get("errors") else 0


if __name__ == "__main__":
    sys.exit(main())
