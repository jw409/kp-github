#!/usr/bin/env python3
"""Aggregate a kp-github usage log (KP_GITHUB_USAGE_LOG) into an optimization view.

The server appends one JSONL record per tool call when KP_GITHUB_USAGE_LOG is set
(call shape + output size only — never arg values). This folds those records into
a per-tool table that surfaces the levers worth pulling:

  fields%   low on a hot tool  -> tighten the tool description or defaults, or have
                                  the calling skill pass `fields` (caller-side win)
  calls / share                -> which descriptions to invest words in
  p50/p95 out_bytes, total     -> where compression work pays off
  err%                         -> a tool whose description invites misuse
  never-called (with KP_GITHUB_BIN) -> registration-string dead weight to trim

Usage:
    python scripts/usage_report.py [path]        # default: $KP_GITHUB_USAGE_LOG or usage.jsonl
    KP_GITHUB_BIN=... python scripts/usage_report.py   # also lists never-called tools
Stdlib only. out_bytes is the serialized result size (a consistent proxy); the
~tok column estimates tokens at ~3.5 bytes/token.
"""
import json, os, sys

BYTES_PER_TOK = 3.5

def pctl(xs, p):
    if not xs:
        return 0
    s = sorted(xs)
    i = min(len(s) - 1, int(round((p / 100) * (len(s) - 1))))
    return s[i]

path = sys.argv[1] if len(sys.argv) > 1 else os.environ.get("KP_GITHUB_USAGE_LOG", "usage.jsonl")
if not os.path.exists(path):
    raise SystemExit(f"no usage log at {path} (set KP_GITHUB_USAGE_LOG or pass a path)")

tools = {}
total = 0
for line in open(path):
    line = line.strip()
    if not line:
        continue
    try:
        r = json.loads(line)
    except json.JSONDecodeError:
        continue
    t = tools.setdefault(r.get("tool", "?"),
                         {"n": 0, "fields": 0, "err": 0, "ok_bytes": [], "total_bytes": 0, "ms": []})
    t["n"] += 1
    total += 1
    if r.get("has_fields"):
        t["fields"] += 1
    if r.get("outcome") != "ok":
        t["err"] += 1
    else:
        t["ok_bytes"].append(r.get("out_bytes", 0))
    t["total_bytes"] += r.get("out_bytes", 0)
    t["ms"].append(r.get("dur_ms", 0))

if not total:
    raise SystemExit(f"{path} has no records yet")

hdr = f"{'tool':<28}{'calls':>6}{'share':>7}{'fields%':>8}{'err%':>6}{'p50B':>8}{'p95B':>9}{'~tok50':>8}{'totKB':>8}{'ms':>6}"
print(f"{path}  ({total} calls)\n")
print(hdr)
print("-" * len(hdr))
for name, t in sorted(tools.items(), key=lambda kv: kv[1]["n"], reverse=True):
    n = t["n"]
    p50 = pctl(t["ok_bytes"], 50)
    print(f"{name:<28}{n:>6}{n/total*100:>6.0f}%{t['fields']/n*100:>7.0f}%{t['err']/n*100:>5.0f}%"
          f"{p50:>8}{pctl(t['ok_bytes'],95):>9}{int(p50/BYTES_PER_TOK):>8}{t['total_bytes']/1024:>7.0f}"
          f"{sum(t['ms'])//len(t['ms']):>6}")

errs = sum(t["err"] for t in tools.values())
print("-" * len(hdr))
print(f"{'TOTAL':<28}{total:>6}{'100%':>7}{'':>8}{errs/total*100:>5.0f}%"
      f"{'':>8}{'':>9}{'':>8}{sum(t['total_bytes'] for t in tools.values())/1024:>7.0f}")

# Optional: never-called tools (registration dead weight) if the binary is available.
bin_path = os.environ.get("KP_GITHUB_BIN")
if bin_path and os.path.exists(bin_path):
    import subprocess, select, time
    def tool_names():
        p = subprocess.Popen([bin_path], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                             stderr=subprocess.DEVNULL, text=True, bufsize=1)
        for m in ('{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"r","version":"0"}}}',
                  '{"jsonrpc":"2.0","method":"notifications/initialized"}',
                  '{"jsonrpc":"2.0","id":2,"method":"tools/list"}'):
            p.stdin.write(m + "\n"); p.stdin.flush()
        dl = time.time() + 15
        try:
            while time.time() < dl:
                if not select.select([p.stdout], [], [], dl - time.time())[0]:
                    break
                line = p.stdout.readline()
                if not line:
                    break
                msg = json.loads(line)
                if msg.get("id") == 2:
                    return {t["name"] for t in msg.get("result", {}).get("tools", [])}
        finally:
            p.terminate()
        return set()
    never = sorted(tool_names() - set(tools))
    if never:
        print(f"\nnever called ({len(never)}) — registration-string trim candidates:")
        for n in never:
            print(f"  {n}")
