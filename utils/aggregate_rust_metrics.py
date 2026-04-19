#!/usr/bin/env python3
"""
Aggregates rust-code-analysis-cli JSON output into an LLM-friendly summary.

Usage:
  1. Run: rust-code-analysis-cli -m -p ./src -O json -o ./metrics_raw
  2. Run: python aggregate_rust_metrics.py ./metrics_raw --top 20 > METRICS_SUMMARY.md

The output is a compact Markdown report designed to fit in an LLM context window.
"""

import json
import os
import sys
import argparse
from pathlib import Path
from dataclasses import dataclass, field

# ── Data structures ──────────────────────────────────────────────────────────

@dataclass
class FunctionMetrics:
    file: str
    name: str
    kind: str  # "function", "impl", "trait", "closure", etc.
    start_line: int
    end_line: int
    cyclomatic: float = 0
    cognitive: float = 0
    sloc: int = 0
    ploc: int = 0
    lloc: int = 0
    cloc: int = 0
    halstead_volume: float = 0
    halstead_difficulty: float = 0
    halstead_effort: float = 0
    halstead_bugs: float = 0
    halstead_time: float = 0
    mi_original: float = 0
    mi_sei: float = 0
    mi_visual_studio: float = 0
    nargs: int = 0
    nexits: int = 0
    nom: int = 0

    @property
    def lines(self):
        return self.end_line - self.start_line + 1


@dataclass
class FileMetrics:
    file: str
    total_sloc: int = 0
    total_ploc: int = 0
    total_cloc: int = 0
    total_blank: int = 0
    functions: list = field(default_factory=list)


# ── Extraction ───────────────────────────────────────────────────────────────

def safe_get(d, *keys, default=0):
    """Safely navigate nested dicts."""
    for k in keys:
        if isinstance(d, dict):
            d = d.get(k, default)
        else:
            return default
    return d if d is not None else default


def extract_functions(space, filepath, results):
    """Recursively walk the 'spaces' tree and extract function-level metrics."""
    kind = space.get("kind", "unknown").lower()
    name = space.get("name", "<anonymous>")

    metrics = space.get("metrics", {})
    loc = metrics.get("loc", {})
    halstead = metrics.get("halstead", {})
    mi = metrics.get("mi", {})
    cyclomatic = metrics.get("cyclomatic", {})
    cognitive = metrics.get("cognitive", {})
    nargs = metrics.get("nargs", {})
    nexits = metrics.get("nexits", {})
    nom = metrics.get("nom", {})

    start = safe_get(space, "start_line", default=0)
    end = safe_get(space, "end_line", default=0)

    # rust-code-analysis-cli emits all values as floats; coerce
    # integer-semantic fields so formatting produces "625" not "625.0".
    fn = FunctionMetrics(
        file=filepath,
        name=name,
        kind=kind,
        start_line=int(start),
        end_line=int(end),
        cyclomatic=safe_get(cyclomatic, "sum"),
        cognitive=safe_get(cognitive, "sum"),
        sloc=int(safe_get(loc, "sloc")),
        ploc=int(safe_get(loc, "ploc")),
        lloc=int(safe_get(loc, "lloc")),
        cloc=int(safe_get(loc, "cloc")),
        halstead_volume=safe_get(halstead, "volume"),
        halstead_difficulty=safe_get(halstead, "difficulty"),
        halstead_effort=safe_get(halstead, "effort"),
        halstead_bugs=safe_get(halstead, "bugs"),
        halstead_time=safe_get(halstead, "time"),
        mi_original=safe_get(mi, "mi_original"),
        mi_sei=safe_get(mi, "mi_sei"),
        mi_visual_studio=safe_get(mi, "mi_visual_studio"),
        nargs=int(safe_get(nargs, "total")),
        nexits=int(safe_get(nexits, "sum")),
        nom=int(safe_get(nom, "total")),
    )
    results.append(fn)

    for child in space.get("spaces", []):
        extract_functions(child, filepath, results)


def process_file(json_path, strip_prefix=""):
    """Process a single JSON output file from rust-code-analysis-cli."""
    with open(json_path, "r") as f:
        data = json.load(f)

    filepath = data.get("name", str(json_path))
    if strip_prefix and filepath.startswith(strip_prefix):
        filepath = filepath[len(strip_prefix):]

    functions = []
    # The top-level object IS a space (the file-level space)
    extract_functions(data, filepath, functions)
    return functions


# ── Aggregation & Reporting ──────────────────────────────────────────────────

def generate_report(all_functions, top_n=20):
    lines = []
    w = lines.append

    # Separate file-level vs function-level entries
    file_level = [f for f in all_functions if f.kind == "unit"]
    fn_level = [f for f in all_functions if f.kind not in ("unit",)]

    # ── Project-wide summary
    total_sloc = sum(f.sloc for f in file_level)
    total_ploc = sum(f.ploc for f in file_level)
    total_cloc = sum(f.cloc for f in file_level)
    num_files = len(file_level)
    num_functions = len([f for f in fn_level if f.kind == "function"])

    w("# Code Quality Metrics Summary\n")
    w(f"**Files analyzed:** {num_files}")
    w(f"**Total SLOC:** {total_sloc:,}  |  **PLOC:** {total_ploc:,}  |  **Comments:** {total_cloc:,}")
    w(f"**Functions/methods found:** {num_functions}")

    if total_ploc > 0:
        comment_ratio = total_cloc / total_ploc * 100
        w(f"**Comment ratio:** {comment_ratio:.1f}%")
    w("")

    # ── File-level MI overview
    if file_level:
        mi_scores = [(f.file, f.mi_visual_studio) for f in file_level if f.mi_visual_studio > 0]
        if mi_scores:
            mi_scores.sort(key=lambda x: x[1])
            avg_mi = sum(s for _, s in mi_scores) / len(mi_scores)
            w("## Maintainability Index (Visual Studio scale, 0-100)\n")
            w(f"**Project average MI:** {avg_mi:.1f}")
            if avg_mi >= 20:
                w("Rating: GOOD (>20 = maintainable)\n")
            elif avg_mi >= 10:
                w("Rating: MODERATE (10-20 = somewhat difficult to maintain)\n")
            else:
                w("Rating: LOW (<10 = difficult to maintain)\n")

            sloc_by_file = {f.file: f.sloc for f in file_level}
            w(f"### Lowest MI files (bottom {min(top_n, len(mi_scores))})\n")
            w("| File | MI | SLOC |")
            w("|------|---:|-----:|")
            for path, mi in mi_scores[:top_n]:
                w(f"| `{path}` | {mi:.1f} | {sloc_by_file.get(path, 0)} |")
            w("")

    # ── High complexity functions
    complex_fns = [f for f in fn_level if f.kind == "function" and f.cyclomatic > 0]

    if complex_fns:
        w("## Cyclomatic Complexity Hotspots\n")
        w("Functions with highest cyclomatic complexity (harder to test):\n")
        by_cc = sorted(complex_fns, key=lambda f: f.cyclomatic, reverse=True)[:top_n]
        w("| Function | File | Line | CC | Cognitive | SLOC |")
        w("|----------|------|-----:|---:|----------:|-----:|")
        for f in by_cc:
            w(f"| `{f.name}` | `{f.file}` | {f.start_line} | {f.cyclomatic:.0f} | {f.cognitive:.0f} | {f.sloc} |")
        w("")

        # Stats
        ccs = [f.cyclomatic for f in complex_fns]
        avg_cc = sum(ccs) / len(ccs)
        max_cc = max(ccs)
        over_10 = sum(1 for c in ccs if c > 10)
        over_20 = sum(1 for c in ccs if c > 20)
        w(f"**Avg CC:** {avg_cc:.1f}  |  **Max CC:** {max_cc:.0f}  |  **Functions with CC>10:** {over_10}  |  **CC>20:** {over_20}\n")

    # ── Cognitive complexity
    cognitive_fns = [f for f in fn_level if f.kind == "function" and f.cognitive > 0]
    if cognitive_fns:
        w("## Cognitive Complexity Hotspots\n")
        w("Functions hardest to understand (cognitive complexity):\n")
        by_cog = sorted(cognitive_fns, key=lambda f: f.cognitive, reverse=True)[:top_n]
        w("| Function | File | Line | Cognitive | CC | SLOC |")
        w("|----------|------|-----:|----------:|---:|-----:|")
        for f in by_cog:
            w(f"| `{f.name}` | `{f.file}` | {f.start_line} | {f.cognitive:.0f} | {f.cyclomatic:.0f} | {f.sloc} |")
        w("")

    # ── Halstead effort hotspots
    effort_fns = [f for f in fn_level if f.kind == "function" and f.halstead_effort > 0]
    if effort_fns:
        w("## Halstead Effort Hotspots\n")
        w("Functions requiring the most effort to maintain:\n")
        by_effort = sorted(effort_fns, key=lambda f: f.halstead_effort, reverse=True)[:top_n]
        w("| Function | File | Effort | Volume | Est. Bugs | SLOC |")
        w("|----------|------|-------:|-------:|----------:|-----:|")
        for f in by_effort:
            w(f"| `{f.name}` | `{f.file}` | {f.halstead_effort:,.0f} | {f.halstead_volume:,.0f} | {f.halstead_bugs:.2f} | {f.sloc} |")
        w("")

    # ── Large functions
    large_fns = sorted(
        [f for f in fn_level if f.kind == "function"],
        key=lambda f: f.sloc, reverse=True
    )[:top_n]
    if large_fns:
        w("## Largest Functions by SLOC\n")
        w("| Function | File | Line | SLOC | CC | Cognitive |")
        w("|----------|------|-----:|-----:|---:|----------:|")
        for f in large_fns:
            w(f"| `{f.name}` | `{f.file}` | {f.start_line} | {f.sloc} | {f.cyclomatic:.0f} | {f.cognitive:.0f} |")
        w("")

    # ── Functions with many args
    many_args = sorted(
        [f for f in fn_level if f.kind == "function" and f.nargs > 3],
        key=lambda f: f.nargs, reverse=True
    )[:top_n]
    if many_args:
        w("## Functions With Many Parameters (>3)\n")
        w("| Function | File | Args | SLOC |")
        w("|----------|------|-----:|-----:|")
        for f in many_args:
            w(f"| `{f.name}` | `{f.file}` | {f.nargs} | {f.sloc} |")
        w("")

    # ── Actionable summary
    w("## Actionable Summary\n")
    issues = []
    if complex_fns:
        over_10 = [f for f in complex_fns if f.cyclomatic > 10]
        if over_10:
            issues.append(f"- **{len(over_10)} functions** have cyclomatic complexity >10 (consider splitting)")
    if cognitive_fns:
        hard = [f for f in cognitive_fns if f.cognitive > 15]
        if hard:
            issues.append(f"- **{len(hard)} functions** have cognitive complexity >15 (hard to understand)")
    if large_fns and large_fns[0].sloc > 100:
        big = [f for f in fn_level if f.kind == "function" and f.sloc > 100]
        issues.append(f"- **{len(big)} functions** exceed 100 SLOC (consider decomposing)")
    if many_args:
        issues.append(f"- **{len(many_args)} functions** have >3 parameters (consider struct/builder)")
    if effort_fns:
        high_bugs = [f for f in effort_fns if f.halstead_bugs > 1.0]
        if high_bugs:
            issues.append(f"- **{len(high_bugs)} functions** have Halstead estimated bugs >1.0 (high defect risk)")

    if issues:
        for issue in issues:
            w(issue)
    else:
        w("No major quality concerns detected. Metrics look healthy.")

    w("")
    return "\n".join(lines)


# ── Main ─────────────────────────────────────────────────────────────────────

def main():
    parser = argparse.ArgumentParser(
        description="Aggregate rust-code-analysis-cli JSON into LLM-friendly Markdown"
    )
    parser.add_argument("input_dir", help="Directory containing JSON metric files")
    parser.add_argument("--top", type=int, default=20, help="Number of top items per category (default: 20)")
    parser.add_argument("--strip-prefix", default="", help="Strip this prefix from file paths")
    parser.add_argument("--output", "-o", help="Output file (default: stdout)")
    args = parser.parse_args()

    input_dir = Path(args.input_dir)
    if not input_dir.is_dir():
        print(f"Error: {input_dir} is not a directory", file=sys.stderr)
        sys.exit(1)

    json_files = sorted(input_dir.rglob("*.json"))
    if not json_files:
        print(f"Error: no JSON files found in {input_dir}", file=sys.stderr)
        sys.exit(1)

    all_functions = []
    errors = []
    for jf in json_files:
        try:
            fns = process_file(jf, strip_prefix=args.strip_prefix)
            all_functions.extend(fns)
        except Exception as e:
            errors.append(f"  Warning: failed to process {jf}: {e}")

    if errors:
        for err in errors:
            print(err, file=sys.stderr)

    report = generate_report(all_functions, top_n=args.top)

    if args.output:
        with open(args.output, "w") as f:
            f.write(report)
        print(f"Report written to {args.output}", file=sys.stderr)
    else:
        print(report)


if __name__ == "__main__":
    main()
