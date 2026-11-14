#!/usr/bin/env python3
"""Retrieval eval: load notes_eval corpus into a temp memhub-mcp DB over MCP stdio,
run queries through search_notes, report nDCG@10 / Recall@10 / MRR@10. Stdlib only."""
import argparse
import json
import math
import os
import subprocess
import sys
import tempfile
import time
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DATA = ROOT / "eval" / "notes_eval"
K = 10


class Mcp:
    def __init__(self, binary, env):
        self.p = subprocess.Popen([str(binary)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  env=env, text=True, bufsize=1)
        self.n = 0
        self.rpc("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                                "clientInfo": {"name": "run_eval", "version": "1"}})
        self.send({"jsonrpc": "2.0", "method": "notifications/initialized"})

    def send(self, msg):
        self.p.stdin.write(json.dumps(msg) + "\n")
        self.p.stdin.flush()

    def rpc(self, method, params):
        self.n += 1
        self.send({"jsonrpc": "2.0", "id": self.n, "method": method, "params": params})
        while True:
            line = self.p.stdout.readline()
            if not line:
                sys.exit(f"server closed during {method}")
            msg = json.loads(line)
            if msg.get("id") == self.n:
                if "error" in msg:
                    sys.exit(f"{method}: {msg['error']}")
                return msg["result"]

    def tool(self, name, args):
        res = self.rpc("tools/call", {"name": name, "arguments": args})
        text = res["content"][0]["text"]
        if res.get("isError"):
            sys.exit(f"{name}: {text}")
        return json.loads(text)

    def close(self):
        self.p.stdin.close()
        self.p.wait(timeout=10)


def load_jsonl(path):
    return [json.loads(l) for l in path.read_text().splitlines() if l.strip()]


def load_qrels():
    q = defaultdict(dict)
    for line in (DATA / "qrels.tsv").read_text().splitlines()[1:]:
        qid, cid, s = line.split("\t")
        q[qid][cid] = int(s)
    return q


def metrics(ranked, rel):
    gains = [rel.get(c, 0) for c in ranked[:K]]
    dcg = sum((2 ** g - 1) / math.log2(i + 2) for i, g in enumerate(gains))
    ideal = sorted((g for g in rel.values() if g > 0), reverse=True)[:K]
    idcg = sum((2 ** g - 1) / math.log2(i + 2) for i, g in enumerate(ideal))
    relevant = {c for c, g in rel.items() if g > 0}
    recall = len(relevant & set(ranked[:K])) / len(relevant) if relevant else 0.0
    mrr = next((1 / (i + 1) for i, g in enumerate(gains) if g > 0), 0.0)
    return (dcg / idcg if idcg else 0.0), recall, mrr


def main():
    ap = argparse.ArgumentParser(epilog="Exit 0 unless --baseline is given and the non-inferiority gate fails (exit 1).")
    ap.add_argument("--split", choices=["dev", "test"], default="dev")
    ap.add_argument("--binary", default=str(ROOT / "target" / "release" / "memhub-mcp"))
    ap.add_argument("--per-query", help="write per-query metrics as JSONL to this path")
    ap.add_argument("--baseline", help="--per-query file of an earlier run; exit 1 unless nDCG@10 is non-inferior")
    ap.add_argument("--margin", type=float, default=0.02, help="allowed nDCG@10 drop vs --baseline (one-sided TOST bound)")
    args = ap.parse_args()

    tmp = tempfile.mkdtemp(prefix="memhub_mcp_eval_")
    env = dict(os.environ, MEMHUB_MCP_DB_PATH=f"{tmp}/memory.db",
               MEMHUB_MCP_SOCKET_PATH=f"{tmp}/daemon.sock", MEMHUB_MCP_IDLE_EXIT_SECS="2")
    mcp = Mcp(args.binary, env)
    id_to_cid = {}
    for row in load_jsonl(DATA / "corpus.jsonl"):
        out = mcp.tool("write_note", {"title": row["title"], "body": row["body"], "tags": row["tags"]})
        id_to_cid[out["id"] if isinstance(out, dict) else out] = row["id"]

    deadline = time.monotonic() + 60
    while mcp.tool("search_notes", {"query": row["title"], "limit": 1}).get("pending_embeddings", 0) > 0:
        if time.monotonic() > deadline:
            sys.exit("embeddings still pending after 60 s")
        time.sleep(0.5)

    qrels = load_qrels()
    queries = [q for q in load_jsonl(DATA / "queries.jsonl") if q["split"] == args.split]
    sums = defaultdict(lambda: [0.0, 0.0, 0.0, 0])
    keyword_only = set()
    per_query = []
    for q in queries:
        res = mcp.tool("search_notes", {"query": q["text"], "limit": K})
        keyword_only.add(res.get("keyword_only", True))
        ranked = [id_to_cid[h["note_id"]] for h in res["hits"]]
        m = metrics(ranked, qrels.get(q["id"], {}))
        per_query.append({"id": q["id"], "kind": q["kind"], "ndcg10": m[0], "recall10": m[1], "mrr10": m[2]})
        for key in ("all", q["kind"]):
            s = sums[key]
            s[0] += m[0]; s[1] += m[1]; s[2] += m[2]; s[3] += 1
    mcp.close()
    per_query_path = args.per_query or (args.baseline and f"{tmp}/per_query.jsonl")
    if per_query_path:
        Path(per_query_path).write_text("".join(json.dumps(r) + "\n" for r in per_query))

    print(f"split={args.split} queries={len(queries)} keyword_only={sorted(keyword_only)}")
    print(f"{'subset':<12}{'n':>5}{'nDCG@10':>10}{'R@10':>8}{'MRR@10':>9}")
    for key, (n, r, m, c) in sorted(sums.items()):
        print(f"{key:<12}{c:>5}{n / c:>10.4f}{r / c:>8.4f}{m / c:>9.4f}")
    if args.baseline:
        sys.exit(baseline_gate(per_query_path, args.baseline, args.margin))


def baseline_gate(current, baseline, margin):  # 0 if current nDCG@10 >= baseline - margin (90% CI), else 1
    out = subprocess.check_output([sys.executable, str(ROOT / "eval" / "paired_bootstrap.py"), current, baseline,
                                   "--metric", "ndcg10", "--margin", str(margin)], text=True)
    ok = f"non-inferior(A>=B-{margin})=True" in out
    print(out + f"baseline gate {'PASS' if ok else 'FAIL'}: nDCG@10 vs {baseline}, margin {margin}")
    return int(not ok)


if __name__ == "__main__":
    main()
