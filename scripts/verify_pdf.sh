#!/usr/bin/env bash
# PDF end-to-end verification for kzktdk (idempotent, informational).
#
# What it does:
#   1. Reports how `bind_pdfium()` (src/archive.rs) will resolve the native
#      Pdfium library: PDFIUM_LIB_PATH -> system library.
#   2. Runs the `pdf_roundtrip_or_skip` unit test, which performs a full
#      create_pdf -> extract_pdf_pages -> create_pdf round-trip on two
#      synthetic PNGs when a library is present, and soft-skips otherwise.
#   3. Prints a final PDF_OK / PDF_SKIP banner.
#
# Needs no LLM keys and no network. Exit code is always 0; read the banner.
set -u
cd "$(dirname "$0")/.."

echo "=== [1/2] Pdfium library resolution ==="
echo "PDFIUM_LIB_PATH=${PDFIUM_LIB_PATH:-<unset>}"
found=""
if [ -n "${PDFIUM_LIB_PATH:-}" ] && [ -f "$PDFIUM_LIB_PATH" ]; then
  found="$PDFIUM_LIB_PATH (via PDFIUM_LIB_PATH)"
fi
for cand in \
  /usr/lib/libpdfium.so /usr/local/lib/libpdfium.so \
  /usr/lib/x86_64-linux-gnu/libpdfium.so \
  /opt/homebrew/lib/libpdfium.dylib /usr/local/lib/libpdfium.dylib; do
  if [ -f "$cand" ]; then
    found="$cand (system)"
    break
  fi
done
if command -v ldconfig >/dev/null 2>&1; then
  sys="$(ldconfig -p 2>/dev/null | grep -m1 libpdfium || true)"
  [ -n "$sys" ] && found="$sys (ldconfig)"
fi
if [ -n "$found" ]; then
  echo "library: $found"
else
  echo "library: NOT FOUND (see docs/PDFIUM.md for per-OS install)"
fi

echo "=== [2/2] Round-trip test (2 synthetic PNGs, no LLM) ==="
out="$(cargo test --lib pdf_roundtrip_or_skip -- --nocapture 2>&1)"
echo "$out" | grep -E "test result|SKIP pdf_roundtrip|running" | head -5

echo "=== result ==="
if echo "$out" | grep -q "SKIP pdf_roundtrip"; then
  echo "PDF_SKIP: no Pdfium library; PDF commands will fail gracefully (exit 1 + message)."
elif echo "$out" | grep -q "test result: ok"; then
  echo "PDF_OK: create_pdf -> extract_pdf_pages -> create_pdf round-trip passed."
else
  echo "PDF_FAIL: unexpected test output; inspect above."
fi
