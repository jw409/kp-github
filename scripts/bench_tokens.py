#!/usr/bin/env python3
"""kp-github token-compression benchmark (canonical, true-token measurement).

Compressed side: drives the real kp-github-mcp binary over MCP stdio.
Raw side:        `gh api` on the identical endpoint (what the official GitHub
                 plugin feeds into context verbatim).
Count:           tiktoken cl100k_base. The *ratio* is tokenizer-robust — the
                 compression is structural (dropped fields), so byte and token
                 ratios track ~1:1 and the number transfers across tokenizers.

Runs against a public repo (cli/cli) so the measurement carries no private data
and anyone can reproduce it. The Rust test `test_compression_floors` enforces
byte-ratio floors in CI; this script is the human-readable token report.

Usage:
    cargo build --release            # from mcp-servers/github
    uv run --with tiktoken python scripts/bench_tokens.py
    # or:  pip install tiktoken && python scripts/bench_tokens.py
Requires an authenticated `gh` (or GITHUB_TOKEN in env).
"""
import json, os, select, subprocess, time

HERE = os.path.dirname(os.path.abspath(__file__))
BIN = next((p for p in (
    os.environ.get("KP_GITHUB_BIN", ""),
    os.path.join(HERE, "..", "target", "release", "kp-github-mcp"),
    os.path.join(HERE, "..", "target", "x86_64-unknown-linux-gnu", "release", "kp-github-mcp"),
) if p and os.path.exists(p)), None)
if BIN is None:
    raise SystemExit("build the server first: `cargo build --release` in mcp-servers/github")

OWNER, REPO = "cli", "cli"

import tiktoken
ENC = tiktoken.get_encoding("cl100k_base")
def toks(s): return len(ENC.encode(s))

def gh_api(path):
    r = subprocess.run(["gh", "api", path], capture_output=True, text=True)
    return (r.stdout if r.returncode == 0 else "")

def mcp_call(tool, args, timeout=40):
    """Spawn the binary, MCP handshake, one tools/call, return the text content."""
    env = dict(os.environ)
    env.setdefault("GITHUB_TOKEN",
                   subprocess.run(["gh", "auth", "token"], capture_output=True, text=True).stdout.strip())
    p = subprocess.Popen([BIN], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                         stderr=subprocess.DEVNULL, text=True, env=env, bufsize=1)
    def send(o): p.stdin.write(json.dumps(o) + "\n"); p.stdin.flush()
    send({"jsonrpc": "2.0", "id": 1, "method": "initialize",
          "params": {"protocolVersion": "2024-11-05", "capabilities": {},
                     "clientInfo": {"name": "bench", "version": "0"}}})
    send({"jsonrpc": "2.0", "method": "notifications/initialized"})
    send({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
          "params": {"name": tool, "arguments": args}})
    deadline = time.time() + timeout
    try:
        while time.time() < deadline:
            if not select.select([p.stdout], [], [], deadline - time.time())[0]:
                break
            line = p.stdout.readline()
            if not line:
                break
            msg = json.loads(line)
            if msg.get("id") == 2:
                content = msg.get("result", {}).get("content", [])
                return "".join(c.get("text", "") for c in content if c.get("type") == "text")
    finally:
        p.terminate()
    return ""

def latest_pr():
    return json.loads(gh_api(f"/repos/{OWNER}/{REPO}/pulls?state=all&per_page=1"))[0]["number"]

def row(label, comp, raw):
    rt, ct = toks(raw), max(toks(comp), 1)
    print(f"{label:<26}{rt:>9}{ct:>10}{rt/ct:>7.1f}x")
    return rt, ct

pr = latest_pr()
print(f"repo={OWNER}/{REPO}  PR#={pr}  tokenizer=cl100k_base\n")
print(f"{'endpoint':<26}{'raw_tok':>9}{'comp_tok':>10}{'tok_x':>8}")
print("-" * 53)

# General read endpoints (defaults, no `fields` filter).
tot_r = tot_c = 0
for label, tool, args, path in [
    ("issues_list(5)", "github_issues_list",
        {"owner": OWNER, "repo": REPO, "limit": 5}, f"/repos/{OWNER}/{REPO}/issues?per_page=5"),
    ("prs_list(5)", "github_prs_list",
        {"owner": OWNER, "repo": REPO, "state": "open", "limit": 5},
        f"/repos/{OWNER}/{REPO}/pulls?per_page=5&state=open"),
    (f"prs_get(#{pr})", "github_prs_get",
        {"owner": OWNER, "repo": REPO, "number": pr}, f"/repos/{OWNER}/{REPO}/pulls/{pr}"),
    (f"prs_files(#{pr})", "github_prs_files",
        {"owner": OWNER, "repo": REPO, "number": pr}, f"/repos/{OWNER}/{REPO}/pulls/{pr}/files"),
]:
    r, c = row(label, mcp_call(tool, args), gh_api(path))
    tot_r += r; tot_c += c

# Branch tracking: the same compare, three ways. Raw baseline is the full
# compare payload you'd otherwise pull to learn ahead/behind.
print("-" * 53)
print("# branch tracking: work AVOIDED vs a full compare, not same-payload compression")
cmp_path = f"/repos/{OWNER}/{REPO}/compare/trunk~30...trunk"
raw_cmp = gh_api(cmp_path)
row("compare(default)", mcp_call("github_repos_compare",
    {"owner": OWNER, "repo": REPO, "base": "trunk~30", "head": "trunk"}), raw_cmp)
row("compare(fields=status..)", mcp_call("github_repos_compare",
    {"owner": OWNER, "repo": REPO, "base": "trunk~30", "head": "trunk",
     "fields": ["status", "ahead_by", "behind_by", "total_commits"]}), raw_cmp)
row("branch_status", mcp_call("github_branch_status",
    {"owner": OWNER, "repo": REPO, "branch": "trunk", "base": "trunk~30"}), raw_cmp)

print("-" * 53)
print(f"{'read-endpoint blended':<26}{tot_r:>9}{tot_c:>10}{tot_r/max(tot_c,1):>7.1f}x")
