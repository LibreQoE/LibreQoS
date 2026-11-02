#!/usr/bin/env python3
"""Health-check one LibreQoS node's local API.

Usage:
    healthcheck.py <shaper IP address>                  # key from LQOS_API_KEY
    healthcheck.py <shaper IP address> --api-key <key>

The key is a local API key created in the node manager under
**License & Services -> Local API Keys**; it is the same `x-bearer` value every
`/api/v3` request needs. Prefer the environment variable: a key passed on the
command line ends up in your shell history and in process listings.
"""

import argparse
import json
import os
import sys

import requests


def parse_args(argv):
    parser = argparse.ArgumentParser(
        description="Health-check a LibreQoS node's local API.",
    )
    parser.add_argument(
        "shaper_ip",
        help="IP address or hostname of the LibreQoS node.",
    )
    parser.add_argument(
        "--api-key",
        default=os.environ.get("LQOS_API_KEY"),
        help="Local API key (defaults to the LQOS_API_KEY environment variable).",
    )
    return parser.parse_args(argv)


def main(argv):
    args = parse_args(argv)
    if not args.api_key:
        print("Error: no API key. Pass --api-key or set LQOS_API_KEY.", file=sys.stderr)
        return 1

    url = f"http://{args.shaper_ip}:9123/api/v3/health"

    try:
        response = requests.get(url, headers={"x-bearer": args.api_key}, timeout=5)
    except requests.exceptions.Timeout:
        print("Error: The request timed out.")
        return 1
    except requests.exceptions.RequestException as e:
        print(f"Error: An error occurred: {e}")
        return 1

    if response.status_code != 200:
        print(f"Error: Received status code {response.status_code}")
        print(response.text)
        return 1

    print(json.dumps(response.json(), indent=4))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
