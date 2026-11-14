#!/usr/bin/env python3
"""Paired comparison of two run_eval.py --per-query files: mean diff (A - B), paired
bootstrap 95% CI, two-sided sign-flip p-value, TOST against a margin. Stdlib only."""
import argparse
import json
import random


def load(path, metric):
    return {r["id"]: r[metric] for r in map(json.loads, open(path)) if r}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("a")
    ap.add_argument("b")
    ap.add_argument("--metric", default="ndcg10")
    ap.add_argument("--resamples", type=int, default=10000)
    ap.add_argument("--margin", type=float, default=0.02)
    ap.add_argument("--seed", type=int, default=0)
    args = ap.parse_args()

    a, b = load(args.a, args.metric), load(args.b, args.metric)
    if a.keys() != b.keys():
        raise SystemExit("query ids differ between files")
    ids = sorted(a)
    d = [a[i] - b[i] for i in ids]
    n = len(d)
    mean = sum(d) / n
    rng = random.Random(args.seed)
    boots = sorted(sum(rng.choices(d, k=n)) / n for _ in range(args.resamples))
    lo, hi = boots[int(0.025 * args.resamples)], boots[int(0.975 * args.resamples) - 1]
    lo90, hi90 = boots[int(0.05 * args.resamples)], boots[int(0.95 * args.resamples) - 1]
    extreme = sum(abs(sum(x if rng.random() < 0.5 else -x for x in d)) / n >= abs(mean) - 1e-12
                  for _ in range(args.resamples))
    p = (extreme + 1) / (args.resamples + 1)
    print(f"n={n} A={sum(a.values()) / n:.4f} B={sum(b.values()) / n:.4f} diff={mean:+.4f}")
    print(f"95% CI [{lo:+.4f}, {hi:+.4f}]  sign-flip p={p:.4f}")
    print(f"TOST ±{args.margin}: 90% CI [{lo90:+.4f}, {hi90:+.4f}] "
          f"equivalent={-args.margin < lo90 and hi90 < args.margin}  "
          f"non-inferior(A>=B-{args.margin})={lo90 > -args.margin}")


if __name__ == "__main__":
    main()
