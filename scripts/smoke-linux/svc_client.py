#!/usr/bin/env python3
"""A tiny client for Kora's administrator service (smoke tests only; the panel has its own).

  svc_client.py VERB [ARG...]            one request; prints each progress line, then the final line
  KORA_SOCKET=/run/kora/service.sock     where to connect

Exit code: 0 = the service ran it and it succeeded, 3 = refused (error message), 4 = ran and failed,
5 = no answer. The final line is `RESULT ok=<bool> code=<n>` or `REFUSED <message>`."""
import json
import os
import socket
import sys

sock_path = os.environ.get("KORA_SOCKET", "/run/kora/service.sock")
verb, args = sys.argv[1], sys.argv[2:]
env = {}
if os.environ.get("CFX_LICENSE_KEY"):
    env["CFX_LICENSE_KEY"] = os.environ["CFX_LICENSE_KEY"]
s = socket.socket(socket.AF_UNIX)
s.settimeout(120)
try:
    s.connect(sock_path)
except OSError as e:
    print(f"NO-ANSWER {e}")
    sys.exit(5)
s.sendall((json.dumps({"verb": verb, "args": args, "env": env}) + "\n").encode())
f = s.makefile("r")
for line in f:
    m = json.loads(line)
    t = m.get("type")
    if t == "progress":
        print(m["line"])
    elif t == "result":
        extra = f" message={m['message']!r}" if m.get("message") else ""
        print(f"RESULT ok={str(m['ok']).lower()} code={m['code']}{extra}")
        sys.exit(0 if m["ok"] else 4)
    elif t == "error":
        print(f"REFUSED {m['message']}")
        sys.exit(3)
print("NO-ANSWER connection closed")
sys.exit(5)
