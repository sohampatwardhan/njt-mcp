#!/bin/sh
# Reject files that must not be committed: NJ Transit PDFs, captured responses, and env files.
set -eu
if find . -name '*.pdf' -not -path './target/*' -not -path './.git/*' | grep -q .; then
  echo "PDFs must not be committed"
  exit 1
fi
if git ls-files | grep -E '(^|/)\.env$|tokens\.json$|state\.json$' ; then
  echo "secret or cache files are tracked"
  exit 1
fi
echo "repo check ok"
