#!/usr/bin/env bash
# Read the 64 KB block that holds ADDR N times, the clip left in place, and say
# whether the reads agree and what the byte at ADDR was each time. Called by
# `make block`, which passes everything through the environment.
#
# REF, an image of the chip, adds whether each read matches it. OUT, a
# directory that must not exist yet, keeps the blocks read (blk-1.bin, …).
set -euo pipefail

die() { echo "block: $*" >&2; exit 2; }

[[ -n ${ADDR:-} ]] || die "ADDR= is required, e.g. make block ADDR=0x220351"
[[ -n ${PROGRAMMER:-} ]] || die "PROGRAMMER= is required (the Makefile sets it)"
n=${N:-10}
[[ $n =~ ^[1-9][0-9]*$ ]] || die "N=$n is not a count"
[[ -z ${REF:-} || -f $REF ]] || die "$REF: no such file"
[[ -z ${OUT:-} || ! -e $OUT ]] || die "$OUT exists: refusing to overwrite reads"

addr=$((ADDR))
start=$((addr & ~0xffff))
offset=$((addr - start))
skip=$((start / 65536))

read -r -a flashrom <<<"${FLASHROM:-flashrom}"
chip_args=()
[[ -z ${CHIP:-} ]] || chip_args=(-c "$CHIP")

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
printf '0x%06x:0x%06x blk\n' "$start" $((start + 0xffff)) >"$work/layout"

if [[ -n ${REF:-} ]]; then
    dd if="$REF" of="$work/ref.bin" bs=65536 skip="$skip" count=1 status=none
    [[ $(stat -c %s "$work/ref.bin") -eq 65536 ]] || die "$REF holds no block at 0x$(printf %06x "$start")"
fi

printf 'Block 0x%06x–0x%06x, byte 0x%06x, %d reads\n' "$start" $((start + 0xffff)) "$addr" "$n"
for i in $(seq "$n"); do
    "${flashrom[@]}" -p "$PROGRAMMER" "${chip_args[@]}" -l "$work/layout" -i blk \
        -r "$work/chip-$i.bin" >"$work/log" 2>&1 || { cat "$work/log" >&2; die "read $i failed"; }
    dd if="$work/chip-$i.bin" of="$work/blk-$i.bin" bs=65536 skip="$skip" count=1 status=none
    rm "$work/chip-$i.bin"
    byte=$(od -An -tx1 -j "$offset" -N1 "$work/blk-$i.bin" | tr -d ' ')
    line="  read $i: $byte"
    if [[ -n ${REF:-} ]]; then
        if cmp -s "$work/blk-$i.bin" "$work/ref.bin"; then line+="  = REF"; else line+="  ≠ REF"; fi
    fi
    echo "$line"
done

distinct=$(sha256sum "$work"/blk-*.bin | cut -c1-64 | sort -u | wc -l)
if [[ $distinct -eq 1 ]]; then
    echo "block: the $n reads are identical"
else
    echo "block: $distinct different blocks in $n reads"
fi

if [[ -n ${OUT:-} ]]; then
    mkdir -p "$OUT"
    cp "$work"/blk-*.bin "$OUT"/
    echo "block: reads kept in $OUT"
fi
[[ $distinct -eq 1 ]]
