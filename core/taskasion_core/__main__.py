"""命令行入口:

    python -m taskasion_core serve [--host H] [--port P] [--data-dir D]
"""

from __future__ import annotations

import argparse
import os

from .server import VERSION, serve


def main() -> None:
    parser = argparse.ArgumentParser(prog="taskasion-core", description="Taskasion 本地核心服务")
    sub = parser.add_subparsers(dest="command", required=True)
    serve_p = sub.add_parser("serve", help="启动本地 REST API")
    serve_p.add_argument("--host", default="127.0.0.1")
    serve_p.add_argument("--port", type=int, default=int(os.environ.get("TASKASION_PORT", "14411")))
    serve_p.add_argument("--data-dir", default=os.environ.get("TASKASION_DATA_DIR"))
    args = parser.parse_args()

    if args.command == "serve":
        print(f"taskasion-core {VERSION}")
        serve(host=args.host, port=args.port, data_dir=args.data_dir)


if __name__ == "__main__":
    main()
