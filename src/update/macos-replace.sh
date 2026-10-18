#!/bin/sh
pid="$1"
staged="$2"
target="$3"
while kill -0 "$pid" 2>/dev/null; do
  sleep 1
done
sleep 1

# Prepare the replacement on the installation volume before moving the old app.
work="$(mktemp -d "${target}.update-XXXXXX")" || {
  /usr/bin/open "$target"
  exit 1
}
replacement="$work/replacement.app"
backup="$work/previous.app"
if ! /usr/bin/ditto "$staged" "$replacement" ||
   ! test -x "$replacement/Contents/MacOS/token-usage"; then
  rm -rf "$work"
  /usr/bin/open "$target"
  exit 1
fi
if ! mv "$target" "$backup"; then
  rm -rf "$work"
  /usr/bin/open "$target"
  exit 1
fi
if ! mv "$replacement" "$target"; then
  # If restoration fails, leave the backup in place for manual recovery.
  if mv "$backup" "$target"; then
    rm -rf "$work"
    /usr/bin/open "$target"
  else
    /usr/bin/open "$backup"
  fi
  exit 1
fi
/usr/bin/open "$target"
rm -rf "$work"
rm -f "$0"
