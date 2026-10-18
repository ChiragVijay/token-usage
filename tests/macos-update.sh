#!/bin/sh
# Exercise the production helper with copy/open commands stubbed for portability.
set -eu
helper="$1"
fixture="$(mktemp -d)"
trap 'rm -rf "$fixture"' EXIT HUP INT TERM

for mode in success copy-failure promotion-failure rollback-failure; do
  scenario="$fixture/$mode"
  staged="$scenario/staged app.app"
  target="$scenario/Token Usage.app"
  mkdir -p "$staged/Contents/MacOS" "$target/Contents/MacOS"
  printf '#!/bin/sh\nnew\n' > "$staged/Contents/MacOS/token-usage"
  printf '#!/bin/sh\nold\n' > "$target/Contents/MacOS/token-usage"
  chmod +x "$staged/Contents/MacOS/token-usage"
  script="$scenario/replace.sh"
  cat > "$script" <<'SH'
sleep() { :; }
update_open() { printf '%s\n' "$1" > "$TEST_OPENED"; }
ditto() {
  cp -R "$1" "$2"
  test "$TEST_MODE" != copy-failure
}
mv() {
  case "$TEST_MODE:$1" in
    promotion-failure:*/replacement.app|rollback-failure:*/replacement.app|rollback-failure:*/previous.app)
      return 1 ;;
  esac
  command mv "$@"
}
SH
  sed -e 's|/usr/bin/ditto|ditto|g' -e 's|/usr/bin/open|update_open|g' "$helper" >> "$script"
  if TEST_MODE="$mode" TEST_OPENED="$scenario/opened" sh "$script" 2147483647 "$staged" "$target"; then
    test "$mode" = success
  else
    test "$mode" != success
  fi
  case "$mode" in
    success)
      test "$(tail -n 1 "$target/Contents/MacOS/token-usage")" = new
      test "$(cat "$scenario/opened")" = "$target" ;;
    copy-failure|promotion-failure)
      test "$(tail -n 1 "$target/Contents/MacOS/token-usage")" = old
      test "$(cat "$scenario/opened")" = "$target" ;;
    rollback-failure)
      backup="$(cat "$scenario/opened")"
      test "$(tail -n 1 "$backup/Contents/MacOS/token-usage")" = old ;;
  esac
  test "$(tail -n 1 "$staged/Contents/MacOS/token-usage")" = new
  if test "$mode" != rollback-failure; then
    test "$(find "$scenario" -name '*.update-*' | wc -l | tr -d ' ')" = 0
  fi
  printf 'PASS: %s\n' "$mode"
done
