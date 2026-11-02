#!/usr/bin/env bash
# Checks a built opendeck-weather binary: what CI's macos job and the
# release workflow run on every binary they produce, kept in one place.
#
#   scripts/check-binary.sh <path> [--mach-o] [--static] [--runner "<cmd>"]
#
#   (always)   it exists and is executable; prints `file`'s description.
#   --mach-o   it is an arm64 Mach-O, and (unless --static) ad-hoc signed.
#   --static   don't run it (a macOS binary checked on Linux).
#   --runner   run it through this command, e.g. "qemu-aarch64 -L /usr/aarch64-linux-gnu"
#              (split on spaces on purpose).
#
# Unless --static, it is started without OpenDeck's arguments and must stop
# at the first missing flag.
set -euo pipefail

usage() {
	echo "usage: $0 <path> [--mach-o] [--static] [--runner \"<cmd>\"]" >&2
	exit 2
}

[ $# -ge 1 ] || usage
bin=$1
shift
mach_o=false
run=true
runner=""
while [ $# -gt 0 ]; do
	case $1 in
	--mach-o) mach_o=true ;;
	--static) run=false ;;
	--runner)
		[ $# -ge 2 ] || usage
		runner=$2
		shift
		;;
	*) usage ;;
	esac
	shift
done

test -x "$bin"
file "$bin"

if $mach_o; then
	# macOS's file says "executable arm64", Linux's "arm64 executable".
	file -b "$bin" | grep -Eq '^Mach-O 64-bit (arm64 executable|executable arm64)'
	if $run; then
		# Apple Silicon kills unsigned binaries; the linker's ad-hoc
		# signature must survive `strip = true`.
		codesign --verify --verbose "$bin"
	fi
fi

if $run; then
	# Without OpenDeck's arguments it stops at the first missing flag,
	# which proves it got as far as main().
	# shellcheck disable=SC2086 # the runner is a command line on purpose
	out=$($runner "$bin" 2>&1 || true)
	echo "$out"
	echo "$out" | grep -q "missing CLI flag: -port"
fi
