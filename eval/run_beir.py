#!/usr/bin/env python3
"""Decision 3 gate on BEIR test splits: exact tier vs turbovec tier (forced by MEMHUB_MCP_EXACT_MAX_CHUNKS) on a temp
DB loaded through write_note. Recall@10 of the tier's fused top-10 vs exact's, nDCG@10 and paired_bootstrap.py; the
pair is then rerun with the keyword index emptied, so that recall is of the vector list alone. Stdlib only."""
import argparse, json, math, os, re, sqlite3, subprocess, sys, tempfile, time, urllib.request, zipfile
from pathlib import Path

from run_eval import ROOT, Mcp, load_jsonl

URL = "https://public.ukp.informatik.tu-darmstadt.de/thakur/BEIR/datasets/{}.zip"
K, GATE_RECALL, MARGIN, MAX_TITLE, EMBED_TIMEOUT = 10, 0.98, 0.01, 200, 3600
MODES = (("exact", False, "exact"), ("tier", True, "exact"),  # (mode, tier forced, mode compared with)
         ("exact_vec", False, "exact_vec"), ("tier_vec", True, "exact_vec"))  # *_vec: keyword index emptied


def fetch(root, name):  # downloads and unzips the BEIR archive if missing
    archive = root / f"{name}.zip"
    if not (root / name / "corpus.jsonl").exists():
        if not archive.exists():
            root.mkdir(parents=True, exist_ok=True)
            urllib.request.urlretrieve(URL.format(name), archive)
        zipfile.ZipFile(archive).extractall(root)
    return root / name


def read_split(d):  # corpus, [(qid, text)] judged in qrels/test.tsv, {qid: {doc: grade > 0}}
    qrels = {}
    for qid, doc, grade in (l.split("\t") for l in (d / "qrels" / "test.tsv").read_text().splitlines()[1:]):
        if int(grade) > 0:
            qrels.setdefault(qid, {})[doc] = int(grade)
    text = {q["_id"]: q["text"] for q in load_jsonl(d / "queries.jsonl")}
    return load_jsonl(d / "corpus.jsonl"), [(q, text[q]) for q in qrels if q in text], qrels


def start_daemon(binary, env, log):  # the socket appears once the vector tier is built
    p = subprocess.Popen([str(binary), "daemon"], env=env, stdin=subprocess.DEVNULL,
                         stdout=subprocess.DEVNULL, stderr=log)
    sock, deadline = Path(env["MEMHUB_MCP_SOCKET_PATH"]), time.monotonic() + 60
    while not sock.exists():
        if p.poll() is not None or time.monotonic() > deadline:
            sys.exit(f"daemon did not start; see {log.name}")
        time.sleep(0.05)
    return p


def load_corpus(mcp, corpus):  # one note per doc; title cut to MAX_TITLE, doc id appended when taken
    ids, seen = {}, set()
    for doc in corpus:
        title = (doc["title"].strip() or doc["_id"])[:MAX_TITLE]
        if title.casefold() in seen:
            title = f"{title[:MAX_TITLE - len(doc['_id']) - 3]} [{doc['_id']}]"
        seen.add(title.casefold())
        out = mcp.tool("write_note", {"title": title, "body": doc["text"]})
        ids[out["id"] if isinstance(out, dict) else out] = doc["_id"]
    return ids


def wait_embedded(mcp):
    deadline = time.monotonic() + EMBED_TIMEOUT
    while (left := mcp.tool("search_notes", {"query": "study", "limit": 1}).get("pending_embeddings", 0)) > 0:
        if time.monotonic() > deadline:
            sys.exit(f"{left} chunks still pending after {EMBED_TIMEOUT} s")
        time.sleep(5)


def search(mcp, queries, ids):  # fused top-K doc ids per query, mean ms per call
    ranked, start = {}, time.monotonic()
    for qid, text in queries:
        res = mcp.tool("search_notes", {"query": text, "limit": K})
        if res.get("keyword_only", True):
            sys.exit(f"query {qid} was keyword-only: vector search is off")
        ranked[qid] = [ids[h["note_id"]] for h in res["hits"]]
    return ranked, (time.monotonic() - start) * 1000 / len(queries)


def ndcg(ranked, rel):  # linear gain, as pytrec_eval (BEIR) computes it
    dcg = sum(rel.get(d, 0) / math.log2(i + 2) for i, d in enumerate(ranked[:K]))
    idcg = sum(g / math.log2(i + 2) for i, g in enumerate(sorted(rel.values(), reverse=True)[:K]))
    return dcg / idcg if idcg else 0.0


def run_dataset(args, name, sha):  # every mode over one dataset; the summary row is appended to summary.jsonl
    corpus, queries, qrels = read_split(fetch(Path(args.data), name))
    tmp = Path(tempfile.mkdtemp(prefix=f"memhub_mcp_beir_{name}_"))
    db_path, ids, runs = tmp / "memory.db", None, {}
    base = dict(os.environ, MEMHUB_MCP_DB_PATH=str(db_path),
                MEMHUB_MCP_SOCKET_PATH=str(tmp / "daemon.sock"), MEMHUB_MCP_IDLE_EXIT_SECS="60")
    for mode, tier, _ in MODES:
        if mode == "exact_vec":  # no keyword hits from here on, so the fused order is the vector order
            sqlite3.connect(db_path, isolation_level=None).execute("INSERT INTO notes_fts(notes_fts) VALUES ('delete-all')")
        env = dict(base, MEMHUB_MCP_EXACT_MAX_CHUNKS=args.tier_max if tier else str(2 ** 40))
        with open(tmp / f"daemon-{mode}.log", "w") as log:
            daemon = start_daemon(args.binary, env, log)
            mcp = Mcp(args.binary, env)
            ids = ids or load_corpus(mcp, corpus)
            wait_embedded(mcp)
            with sqlite3.connect(f"file:{db_path}?mode=ro", uri=True) as db:
                chunks = db.execute("SELECT count(*) FROM chunks WHERE embedding IS NOT NULL").fetchone()[0]
            if chunks <= int(args.tier_max):
                sys.exit(f"{chunks} chunks <= --tier-max {args.tier_max}: the tier run would scan exactly")
            runs[mode] = search(mcp, queries, ids)
            mcp.close()
            daemon.terminate()
            daemon.wait(timeout=30)
    (out := Path(args.out)).mkdir(parents=True, exist_ok=True)
    rows, files = {}, {}
    for mode, _, ref in MODES:
        ranked, exact = runs[mode][0], runs[ref][0]
        rows[mode] = [{"id": q, "ndcg10": ndcg(ranked[q], qrels[q]), "top10": ranked[q],
                       "overlap10": len(set(ranked[q]) & set(exact[q])) / max(len(exact[q]), 1)} for q, _ in queries]
        files[mode] = out / f"{name}-{mode}-{sha}.jsonl"
        files[mode].write_text("".join(json.dumps(r) + "\n" for r in rows[mode]))
    boot = subprocess.check_output([sys.executable, str(ROOT / "eval" / "paired_bootstrap.py"), str(files["tier"]),
                                    str(files["exact"]), "--margin", str(MARGIN)], text=True)
    mean = {m: {k: sum(r[k] for r in rows[m]) / len(queries) for k in ("ndcg10", "overlap10")} for m in rows}
    s = {"dataset": name, "sha": sha, "loadavg": os.getloadavg(), "db": str(tmp), "docs": len(corpus),
         "chunks": chunks, "queries": len(queries), "tier_max": int(args.tier_max),
         "recall10_vs_exact": mean["tier"]["overlap10"], "vector_recall10_vs_exact": mean["tier_vec"]["overlap10"],
         "min_recall10_vs_exact": min(r["overlap10"] for r in rows["tier"]), "bootstrap": boot,
         "ndcg10": {m: mean[m]["ndcg10"] for m in rows}, "ms_per_query": {m: runs[m][1] for m in runs},
         "ci95": re.search(r"95% CI \[(\S+), (\S+)\]", boot).groups(), "tost_equivalent": "equivalent=True" in boot}
    s["gate"] = s["recall10_vs_exact"] >= GATE_RECALL and s["tost_equivalent"]
    with open(out / "summary.jsonl", "a") as f:
        f.write(json.dumps(s) + "\n")
    print(f"{name}: {json.dumps(s)}", file=sys.stderr)
    return s


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("datasets", nargs="*", default=["scifact", "nfcorpus"])
    ap.add_argument("--binary", default=str(ROOT / "target" / "release" / "memhub-mcp"))
    ap.add_argument("--data", default="/tmp/beir", help="BEIR datasets dir; missing ones are downloaded")
    ap.add_argument("--out", default="/tmp/beir/results", help="per-query JSONL and summary.jsonl")
    ap.add_argument("--tier-max", default="1000", help="MEMHUB_MCP_EXACT_MAX_CHUNKS for the tier run")
    args = ap.parse_args()
    sha = subprocess.check_output(["git", "-C", str(ROOT), "rev-parse", "--short", "HEAD"], text=True).strip()
    rows = [run_dataset(args, name, sha) for name in args.datasets]
    print("| dataset | docs | chunks | queries | recall@10 tier vs exact, fused | min | recall@10 vector only | "
          "nDCG@10 exact | nDCG@10 tier | diff [95% CI] | TOST ±0.01 | gate |\n|" + "---|" * 12)
    for s in rows:
        n = s["ndcg10"]
        print(f"| {s['dataset']} | {s['docs']} | {s['chunks']} | {s['queries']} | {s['recall10_vs_exact']:.4f} | "
              f"{s['min_recall10_vs_exact']:.2f} | {s['vector_recall10_vs_exact']:.4f} | {n['exact']:.4f} | "
              f"{n['tier']:.4f} | {n['tier'] - n['exact']:+.4f} [{s['ci95'][0]}, {s['ci95'][1]}] | "
              f"{'equivalent' if s['tost_equivalent'] else 'not shown'} | {'pass' if s['gate'] else 'FAIL'} |")
    sys.exit(gate_exit(rows))


def gate_exit(rows):  # 1 when any dataset's gate failed (recall < GATE_RECALL or TOST not equivalent), else 0
    for s in (s for s in rows if not s["gate"]):
        print(f"gate FAIL {s['dataset']}: recall10_vs_exact={s['recall10_vs_exact']:.4f} (need >= {GATE_RECALL}), "
              f"tost_equivalent={s['tost_equivalent']}", file=sys.stderr)
    return int(not all(s["gate"] for s in rows))


if __name__ == "__main__":
    main()
