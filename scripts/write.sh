#!/usr/bin/env bash
# Erase the chip on the clip, write IMAGE to it, let flashrom verify it, then
# read it back into READBACK. Called by `make write`, which passes everything
# through the environment.
#
# Refuses unless:
#   - A and B are two identical saved reads of the chip (the backup);
#   - IMAGE has the size of those reads;
#   - the chip on the clip has that size and its first 64 KB block is A's (the
#     chip they were read from) or IMAGE's (a write being redone);
#   - the user confirms: types WRITE, or passes CONFIRM=WRITE.
#
# The whole chip is erased before the write, so that every programmed cell is
# programmed again, even where IMAGE equals what the chip holds: flashrom alone
# would skip the blocks that already match.
set -euo pipefail

die() { echo "write: $*" >&2; exit 2; }

[[ -n ${IMAGE:-} && -n ${A:-} && -n ${B:-} && -n ${READBACK:-} ]] ||
    die "IMAGE=, A=, B= and READBACK= are required, e.g.
    make write IMAGE=u53-new.bin A=u53-1.bin B=u53-3.bin READBACK=u53-w1.bin"
[[ -n ${PROGRAMMER:-} ]] || die "PROGRAMMER= is required (the Makefile sets it)"

for f in "$IMAGE" "$A" "$B"; do
    [[ -f $f ]] || die "$f: no such file"
done
[[ ! -e $READBACK ]] || die "$READBACK exists: refusing to overwrite a dump"

cmp -s "$A" "$B" ||
    die "$A and $B differ: two identical reads of the chip must be saved before writing it"

size=$(stat -c %s "$A")
image_size=$(stat -c %s "$IMAGE")
[[ $image_size -eq $size ]] ||
    die "$IMAGE is $image_size bytes, the chip's reads $size: wrong image for this chip"

read -r -a flashrom <<<"${FLASHROM:-flashrom}"
chip_args=()
[[ -z ${CHIP:-} ]] || chip_args=(-c "$CHIP")
run() { "${flashrom[@]}" -p "$PROGRAMMER" "${chip_args[@]}" "$@"; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

echo "Reading the chip's first 64 KB block…"
printf '0x000000:0x00ffff first\n' >"$work/layout"
run -l "$work/layout" -i first -r "$work/chip.bin" >"$work/log" 2>&1 || {
    cat "$work/log" >&2
    die "could not read the chip: check the clip with make probe"
}
chip_size=$(stat -c %s "$work/chip.bin")
[[ $chip_size -eq $size ]] ||
    die "the chip on the clip holds $chip_size bytes, the reads $size: wrong chip"

if cmp -s -n 65536 "$work/chip.bin" "$A"; then
    origin="the backup's ($A)"
elif cmp -s -n 65536 "$work/chip.bin" "$IMAGE"; then
    origin="the image's (written before)"
elif [[ ${FORCE:-} == 1 ]]; then
    origin="neither the backup's nor the image's (FORCE=1)"
else
    die "the chip's first block is neither $A's nor $IMAGE's: wrong chip, or a clip
    badly seated (re-seat it, make probe). If an earlier write stopped half-way,
    the chip no longer matches either: rerun with FORCE=1."
fi

changed=$( (cmp -l "$A" "$IMAGE" || true) | wc -l)

cat <<EOF

Chip on the clip  $size bytes, first block identical to $origin
Backup            $A = $B
                  $(sha256sum "$A" | cut -c1-64)
Image             $IMAGE, $changed bytes differ from the backup
                  $(sha256sum "$IMAGE" | cut -c1-64)
Read-back         $READBACK
Programmer        $PROGRAMMER ${CHIP:+(chip $CHIP)}

The whole chip is erased, then IMAGE is written and verified by flashrom,
then read back. The chip holds nothing between the erase and the write.

EOF

if [[ ${CONFIRM:-} != WRITE ]]; then
    (: </dev/tty) 2>/dev/null || die "no terminal to confirm on: pass CONFIRM=WRITE"
    printf 'Type WRITE to erase this chip and write %s: ' "$IMAGE"
    read -r answer </dev/tty || die "no answer: nothing was written"
    [[ $answer == WRITE ]] || die "not confirmed: nothing was written"
fi

halfway() {
    die "$1 failed: the chip may be blank or half written. The backup is $A.
    Re-seat the clip, make probe, then the same make write with FORCE=1."
}

echo "Erasing…"
run --progress -E || halfway "the erase"
echo "Writing and verifying…"
run --progress -w "$IMAGE" || halfway "the write"
echo "Reading back…"
run --progress -r "$READBACK" || die "the read-back failed; the write itself was verified by flashrom"

sha256sum "$IMAGE" "$READBACK"
cmp "$IMAGE" "$READBACK" || die "the read-back differs from the image"
echo "write: done, the read-back is identical to the image"
