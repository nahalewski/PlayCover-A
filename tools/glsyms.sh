#!/bin/bash
[ -f /tmp/flap_dis.txt ] || bash "$(dirname "$0")/disasm.sh" >/dev/null 2>&1
echo "== GL symbols:"
grep -o "symbol stub for: _\(gl\|EAGL\)[A-Za-z0-9_]*" /tmp/flap_dis.txt | sed 's/symbol stub for: _//' | sort -u | tr '\n' ' '
echo
echo "== count:"
grep -o "symbol stub for: _gl[A-Za-z0-9_]*" /tmp/flap_dis.txt | sed 's/symbol stub for: _//' | sort -u | wc -l
