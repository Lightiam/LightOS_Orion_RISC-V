#!/usr/bin/env bash
# Regenerate assets/splash.txt from a video's final frame (the
# "LightOS Orion" title card in Orion.mp4). The video itself is not
# committed; pass its path as $1. Requires ffmpeg + python3.
set -eu

VIDEO=${1:?usage: make_splash.sh <video.mp4>}
OUT=assets/splash.txt
RAW=$(mktemp)
trap 'rm -f "$RAW"' EXIT

ffmpeg -v error -sseof -0.5 -i "$VIDEO" -frames:v 1 \
    -vf "scale=100:26,format=gray" -f rawvideo -pix_fmt gray -y "$RAW"

python3 - "$RAW" "$OUT" <<'EOF'
import sys
raw, out = sys.argv[1], sys.argv[2]
w, h = 100, 26
data = open(raw, "rb").read()[: w * h]
ramp = " .:-=+*#%@"
lines = [
    "".join(ramp[min(data[y * w + x] * len(ramp) // 256, len(ramp) - 1)] for x in range(w)).rstrip()
    for y in range(h)
]
while lines and not lines[0].strip():
    lines.pop(0)
while lines and not lines[-1].strip():
    lines.pop()
with open(out, "w") as f:
    f.write("\n".join(lines) + "\n\n        LightOS Orion — a LightRail AI system\n")
EOF
echo "wrote $OUT"
