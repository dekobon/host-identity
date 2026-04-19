#!/usr/bin/env bash
# Run rust-code-analysis-cli on the project and display aggregated metrics.
#
# Collects JSON metrics from all Rust source files in the crates/ directory,
# aggregates them via aggregate_rust_metrics.py, and prints the report to
# stdout. Temporary files are cleaned up on exit.
#
# Usage:
#   ./utils/code-metrics.sh [OPTIONS]
#
# Options:
#   -h, --help      Show this help message
#   -t, --top N     Number of top items per category (default: 20)
#   -j, --jobs N    Parallel analysis jobs (default: nproc)
#   -o, --output F  Write report to file instead of stdout
#   -p, --path DIR  Analyze a specific directory (default: crates/)

set -euo pipefail

# ---------------------------------------------------------------------------
# Paths
# ---------------------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
AGGREGATOR="$SCRIPT_DIR/aggregate_rust_metrics.py"

# ---------------------------------------------------------------------------
# Defaults
# ---------------------------------------------------------------------------
TOP_N=20
JOBS="$(nproc 2>/dev/null || echo 4)"
OUTPUT=""
ANALYZE_PATH="$PROJECT_ROOT/crates"

# ---------------------------------------------------------------------------
# Colors
# ---------------------------------------------------------------------------
CYAN='\033[0;36m'
GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m'

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
log_info() { echo -e "${CYAN}[info]${NC}  $*" >&2; }
log_error() { echo -e "${RED}[error]${NC} $*" >&2; }
log_ok() { echo -e "${GREEN}[ok]${NC}    $*" >&2; }

usage() {
	cat <<EOF
Usage: $(basename "$0") [OPTIONS]

Run rust-code-analysis-cli on the project and display aggregated metrics.

Options:
  -h, --help      Show this help message
  -t, --top N     Number of top items per category (default: $TOP_N)
  -j, --jobs N    Parallel analysis jobs (default: $JOBS)
  -o, --output F  Write report to file instead of stdout
  -p, --path DIR  Analyze a specific directory (default: crates/)

Prerequisites:
  rust-code-analysis-cli   cargo install rust-code-analysis-cli
  python3                  With standard library (no pip deps)

Examples:
  $(basename "$0")                     # full project report to stdout
  $(basename "$0") -t 10 -o report.md  # top-10, write to file
  $(basename "$0") -p crates/mpi       # analyze a single crate
EOF
}

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------
while [[ $# -gt 0 ]]; do
	case "$1" in
		-h | --help)
			usage
			exit 0
			;;
		-t | --top)
			TOP_N="$2"
			shift 2
			;;
		-j | --jobs)
			JOBS="$2"
			shift 2
			;;
		-o | --output)
			OUTPUT="$2"
			shift 2
			;;
		-p | --path)
			ANALYZE_PATH="$2"
			shift 2
			;;
		*)
			log_error "unknown option: $1"
			usage >&2
			exit 1
			;;
	esac
done

# ---------------------------------------------------------------------------
# Prerequisite checks
# ---------------------------------------------------------------------------
if ! command -v rust-code-analysis-cli >/dev/null 2>&1; then
	log_error "rust-code-analysis-cli not found"
	echo "  Install: cargo install rust-code-analysis-cli" >&2
	exit 1
fi

if ! command -v python3 >/dev/null 2>&1; then
	log_error "python3 not found"
	exit 1
fi

if [[ ! -f "$AGGREGATOR" ]]; then
	log_error "aggregator script not found: $AGGREGATOR"
	exit 1
fi

if [[ ! -d "$ANALYZE_PATH" ]]; then
	log_error "analysis path does not exist: $ANALYZE_PATH"
	exit 1
fi

# ---------------------------------------------------------------------------
# Temporary directory with cleanup
# ---------------------------------------------------------------------------
METRICS_DIR="$(mktemp -d "${TMPDIR:-/tmp}/rust-metrics-XXXXXX")"
trap 'rm -rf "$METRICS_DIR"' EXIT

# ---------------------------------------------------------------------------
# Run analysis
# ---------------------------------------------------------------------------
log_info "analyzing Rust files in ${ANALYZE_PATH#"$PROJECT_ROOT"/}"
log_info "output format: json, jobs: $JOBS"

rust-code-analysis-cli \
	--metrics \
	--paths "$ANALYZE_PATH" \
	--output-format json \
	--output "$METRICS_DIR" \
	--include "*.rs" \
	--num-jobs "$JOBS" \
	2>&1 | while IFS= read -r line; do
	# Suppress empty lines, pass warnings/errors to stderr
	[[ -n "$line" ]] && echo "  $line" >&2
done

# Count results
JSON_COUNT="$(find "$METRICS_DIR" -name '*.json' -type f 2>/dev/null | wc -l)"
if [[ "$JSON_COUNT" -eq 0 ]]; then
	log_error "no JSON metric files produced — check the analysis path"
	exit 1
fi

log_ok "collected metrics for $JSON_COUNT files"

# ---------------------------------------------------------------------------
# Aggregate and report
# ---------------------------------------------------------------------------
log_info "aggregating metrics (top $TOP_N per category)"

AGGREGATE_ARGS=(
	"$METRICS_DIR"
	--top "$TOP_N"
	--strip-prefix "$PROJECT_ROOT/"
)

if [[ -n "$OUTPUT" ]]; then
	python3 "$AGGREGATOR" "${AGGREGATE_ARGS[@]}" --output "$OUTPUT"
	log_ok "report written to $OUTPUT"
else
	python3 "$AGGREGATOR" "${AGGREGATE_ARGS[@]}"
fi
