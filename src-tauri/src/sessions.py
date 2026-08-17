#!/usr/bin/env python3
"""Reports what the browser on this machine is signed in to.

Reads Chrome's own cookie and history files rather than driving the browser.
That is deliberate and buys three things:

- It works with the browser closed. Cookies live on disk, so a machine that has
  just woken can be asked without starting Chrome first, which the alternative
  would do as a side effect of connecting to it.
- It is one command instead of the six the browser driver needs to guarantee a
  page is up, so this is cheap enough to run whenever an agent takes a turn.
- It cannot leak a session. The value and encrypted_value columns are never
  selected, so no token is read at all, rather than being read and dropped.

Both files are copied before being opened, because Chrome holds a lock on them
while it is running.
"""

import json
import os
import shutil
import sqlite3
import sys
import tempfile
import urllib.parse

# Passed in rather than spelled here. The launcher already names this directory
# absolutely, and a second copy of it written as `~` is a copy that resolves
# against whatever user the command daemon happens to run as: agree with the
# launcher on every machine, or agree with it only on the ones where those two
# users match, silently reporting an empty jar everywhere else.
PROFILE = sys.argv[1]


def rows(name, query):
    """Runs one query against a copy of a Chrome database.

    A profile too new to have the file, or a schema Chrome has since changed, is
    an empty answer rather than a failure: the caller's job is to report what is
    signed in, and "nothing found" degrades better than an error that stops an
    agent's turn.
    """
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
    """Every cookie, as name and flags only. No value is ever read."""
    found = rows(
        "Cookies",
        "SELECT host_key, name, is_httponly, is_persistent FROM cookies",
    )
    return [
        {
            "domain": host,
            "name": name,
            "httpOnly": bool(http_only),
            # Chrome stores the opposite of what the caller reasons about: a
            # cookie that is not persistent is one that dies with the browser.
            "session": not bool(persistent),
        }
        for host, name, http_only, persistent in found
    ]


def visited():
    """Hosts this browser has actually navigated to.

    The discriminator between a site somebody uses and an ad network: a tracker
    sets cookies from inside someone else's page and never appears here.
    """
    hosts = set()
    for (url,) in rows("History", "SELECT url FROM urls"):
        host = urllib.parse.urlparse(url).hostname
        if host:
            hosts.add(host)
    return sorted(hosts)


if __name__ == "__main__":
    print(json.dumps({"cookies": cookies(), "visited": visited()}))
