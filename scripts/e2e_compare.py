#!/usr/bin/env python3
"""Compares tails-e2e --summary files of one index asked with different query-side
configurations (RAG_FUSION, RAG_KEYWORD_STOPWORDS). Used by scripts/e2e.sh when
E2E_COMPARE is set; the first file is the run that decides the exit status.

Prints the aggregates per configuration, then every question whose recall,
precision@R, excluded distractors or ranking margin differs between them, with the
notes of the configurations it fails in. Margin: the lowest reranked score of a
must-retrieve source over the highest score of an unrelated one (above 1: ranked
first).
"""
import json
import sys

METRICS = [
    ("recallAtK", "recall@k"),
    ("precisionAtR", "precision@R"),
    ("distractorExclusion", "distractor exclusion"),
    ("scopeAccuracy", "scope accuracy"),
    ("citationValidity", "citation validity"),
    ("citationAccuracy", "citation accuracy"),
    ("observationRecall", "observation recall"),
    ("negativeControlsFlagged", "negative controls flagged"),
    ("evidenceAccuracy", "evidence accuracy"),
]


def label(s):
    return f"{s['fusion']}/{'stop' if s['stopwords'] else 'nostop'}"


def main(paths):
    runs = []
    for p in paths:
        with open(p) as f:
            runs.append(json.load(f))
    labels = [label(r) + ("*" if i == 0 else "") for i, r in enumerate(runs)]
    width = max(len(l) for l in labels) + 2
    print(f"{'':<28}" + "".join(f"{l:>{width}}" for l in labels))
    for key, name in METRICS:
        print(f"{name:<28}" + "".join(f"{r['aggregate'][key]:>{width}.3f}" for r in runs))
    passed = ["ok" if not r["hard"] and not r["belowThreshold"] else "FAIL" for r in runs]
    print(f"{'hard checks + thresholds':<28}" + "".join(f"{p:>{width}}" for p in passed))

    def cell(q):
        if q is None:
            return "-"
        margin = "-" if q["margin"] is None else f"{q['margin']:.3f}"
        return f"{q['recall']:.2f}/{q['precision']:.2f}/{q['excluded']}/{margin}"

    by_id = [{q["id"]: q for q in r["questions"]} for r in runs]
    ids = list(by_id[0]) + [i for b in by_id[1:] for i in b if i not in by_id[0]]
    print("\nquestions that differ (recall/prec@R/distractors excluded/margin):")
    differ = False
    for qid in ids:
        cells = [cell(b.get(qid)) for b in by_id]
        if len(set(cells)) == 1:
            continue
        differ = True
        print(f"  {qid}")
        for l, c, b in zip(labels, cells, by_id):
            problems = "; ".join((b.get(qid) or {}).get("problems", []))
            print(f"    {l:<{width}} {c:<24} {problems}")
    if not differ:
        print("  none")


if __name__ == "__main__":
    main(sys.argv[1:])
