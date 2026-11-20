#!/usr/bin/env python3
"""Launcher: python netgd-send.py {prepare,collect,status} ...  (same as python -m netgd_send)."""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from netgd_send.__main__ import main  # noqa: E402

if __name__ == "__main__":
    sys.exit(main())
