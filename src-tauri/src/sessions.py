#!/usr/bin/env python3

import json
import os
import shutil
import sqlite3
import sys
import tempfile
import urllib.parse

PROFILE = sys.argv[1]


def rows(name, query):
    source = os.path.join(PROFILE, name)
    if not os.path.exists(source):
        return []
    copy = os.path.join(tempfile.gettempdir(), f"guac-{name.lower()}")
    try:
        shutil.copyfile(source, copy)
        db = sqlite3.connect(copy)
        found = db.execute(query).fetchall()
        db.close()
        return found
    except Exception:
        return []


def cookies():
    found = rows(
        "Cookies",
        "SELECT host_key, name, is_httponly, is_persistent FROM cookies",
    )
    return [
        {
            "domain": host,
            "name": name,
            "httpOnly": bool(http_only),
            "session": not bool(persistent),
        }
        for host, name, http_only, persistent in found
    ]


def visited():
    hosts = set()
    for (url,) in rows("History", "SELECT url FROM urls"):
        host = urllib.parse.urlparse(url).hostname
        if host:
            hosts.add(host)
    return sorted(hosts)


if __name__ == "__main__":
    print(json.dumps({"cookies": cookies(), "visited": visited()}))
