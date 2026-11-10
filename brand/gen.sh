#!/bin/bash
# gpt-image-2 via Codex: gen.sh <out.png> <prompt.txt> <size words> [image1 ...]
set -e
B=/Users/sompoteyouwai/ENV4/ddepsse_tellus/tellushive/brand
out=$1; pf=$2; size=$3; shift 3
imgs=(); for f in "$@"; do imgs+=(-i "$f"); done
{ echo "Use your image generation tool (gpt-image-2) to create ONE image at the highest quality and a $size size, then copy the generated PNG to $out and reply with only that path. Do not write any code or other files."; echo; cat "$pf"; } |
  codex exec -m gpt-6-sol --skip-git-repo-check -s workspace-write -C "$B" "${imgs[@]}" - 2>&1 | tail -2
