#!/usr/bin/env bash
# Compare two reads of a chip, or a read and an image: their sizes and
# digests, then each byte that differs, its address and both values, so that a
# weak cell read two ways shows as one known address. Called by `make
# compare`, which passes A and B through the environment.
#
# Exits 0 when the two files are identical, 1 when they differ.
set -euo pipefail

die() { echo "compare: $*" >&2; exit 2; }

[[ -n ${A:-} && -n ${B:-} ]] || die "A= and B= are required, e.g. make compare A=u52-1.bin B=u52-2.bin"
for f in "$A" "$B"; do
    [[ -f $f ]] || die "$f: no such file"
done
shown=${SHOW:-20}

sha256sum "$A" "$B"
size_a=$(stat -c %s "$A")
size_b=$(stat -c %s "$B")
[[ $size_a -eq $size_b ]] || { echo "compare: sizes differ, $size_a and $size_b bytes"; exit 1; }

if cmp -s "$A" "$B"; then
    echo "compare: identical, $size_a bytes"
    exit 0
fi

# cmp -l numbers bytes from 1 and prints the values in octal.
diffs=$(cmp -l "$A" "$B" || true)
count=$(wc -l <<<"$diffs")
echo "compare: $count bytes differ of $size_a"
printf '  %-10s %-4s %-4s\n' address A B
head -n "$shown" <<<"$diffs" | while read -r offset a b; do
    printf '  0x%06x   %02X   %02X\n' $((offset - 1)) $((8#$a)) $((8#$b))
done
[[ $count -le $shown ]] || echo "  … $((count - shown)) more (SHOW=$count to list them all)"
exit 1
