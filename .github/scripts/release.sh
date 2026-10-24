#!/usr/bin/env bash
# Builds the release binary for one target, packs it and checks the archive for paths of the machine that built it (M9).
# Run by .github/workflows/release.yml; also runnable by hand from the repository root:
#   bash .github/scripts/release.sh <rust target> <version name>
# Result: dist/zcrowd-<version name>-<target>.tar.gz (.zip for Windows targets).
set -euo pipefail

# One forbidden string per line. Built from pieces so that this file does not contain them itself.
forbidden() {
  local p
  printf '%s\n' "/ho""me/" "/Us""ers/" "C:\\Us""ers\\"
  # The builder's own directories, in the spellings a Windows toolchain may write.
  for p in "${GITHUB_WORKSPACE:-$PWD}" "${CARGO_HOME:-$HOME/.cargo}" "${RUSTUP_HOME:-$HOME/.rustup}"; do
    printf '%s\n' "$p"
    if command -v cygpath >/dev/null; then
      cygpath -w "$p"
      cygpath -m "$p"
    fi
  done
}

# $1 = directory. Exits 1 and says which file holds which string if anything forbidden is inside (binary files included).
check_clean() {
  local list files f
  list=$(mktemp)
  forbidden | sed '/^$/d' >"$list"
  files=$(grep -rlaiF -f "$list" "$1" || true)
  if [ -n "$files" ]; then
    echo "FAIL: a path of the build machine is in what would be published:" >&2
    echo "$files" | while IFS= read -r f; do
      echo "  $f: $(grep -aoiF -f "$list" "$f" | sort | uniq -c | tr '\n' ' ')" >&2
    done
    rm -f "$list"
    return 1
  fi
  rm -f "$list"
}

# $1 = binary, $2 = target. Prints the shared libraries the binary needs and fails on one the user's machine may not have.
check_links() {
  local bad=""
  case $2 in
    *linux*)
      ldd "$1"
      # Only glibc (SQLite is built in). Anything else is a decision for a person.
      bad=$(ldd "$1" | awk '{print $1}' | grep -Ev '^(linux-vdso\.so\.1|/lib64/ld-linux-x86-64\.so\.2|libc\.so\.6|libm\.so\.6|libgcc_s\.so\.1)$' || true)
      ;;
    *apple*)
      otool -L "$1"
      bad=$(otool -L "$1" | tail -n +2 | awk '{print $1}' | grep -Ev '^(/usr/lib/|/System/Library/)' || true)
      ;;
  esac
  if [ -n "$bad" ]; then
    echo "FAIL: the binary needs libraries that are not part of the system:" >&2
    echo "$bad" >&2
    return 1
  fi
}

# Maps the cargo home, rustup home and workspace to fixed names, so panic messages and debug paths hold none of them.
remap_flags() {
  local home=${CARGO_HOME:-$HOME/.cargo} rustup_home=${RUSTUP_HOME:-$HOME/.rustup} ws=${GITHUB_WORKSPACE:-$PWD}
  if command -v cygpath >/dev/null; then
    home=$(cygpath -w "$home")
    rustup_home=$(cygpath -w "$rustup_home")
    ws=$(cygpath -w "$ws")
  fi
  echo "--remap-path-prefix=$home=/cargo --remap-path-prefix=$rustup_home=/rustup --remap-path-prefix=$ws=/src"
}

main() {
  local target=${1:?rust target} version=${2:?version name}
  local ext="" name bin out archive
  case $target in *windows*) ext=.exe ;; esac
  name=zcrowd-$version-$target
  bin=target/$target/release/zcrowd$ext

  # LZMA_API_STATIC: build liblzma from the bundled source, whatever the builder has installed (Homebrew's xz would be linked by path).
  RUSTFLAGS=$(remap_flags) LZMA_API_STATIC=1 cargo build --locked --release --target "$target" -p zcrowd

  # The binary must start and answer on its own target (a missing library shows up here).
  "$bin" --version
  out=$(printf '1.0\n' | "$bin" check --unshield - --data fixtures/flows-sample.ndjson 2>&1) || { echo "$out"; exit 1; }
  echo "$out"
  echo "$out" | grep -E '^verdict: (NO MATCH|RED|AMBER|GREEN)' >/dev/null
  check_links "$bin" "$target"

  rm -rf dist
  mkdir -p "dist/stage/$name" dist/check
  cp "$bin" LICENSE README.md "dist/stage/$name/"
  check_clean dist/stage

  case $target in
    *windows*)
      # shellcheck disable=SC2016  # $env:NAME is PowerShell syntax, not bash
      NAME=$name pwsh -NoProfile -Command 'Compress-Archive -Path "dist/stage/$env:NAME" -DestinationPath "dist/$env:NAME.zip"'
      # shellcheck disable=SC2016
      NAME=$name pwsh -NoProfile -Command 'Expand-Archive -Path "dist/$env:NAME.zip" -DestinationPath dist/check'
      archive=$name.zip
      ;;
    *)
      tar -czf "dist/$name.tar.gz" -C dist/stage "$name"
      tar -xzf "dist/$name.tar.gz" -C dist/check
      archive=$name.tar.gz
      ;;
  esac
  # Look again at what a user will unpack, and make sure it is the binary that was just tested.
  cmp "$bin" "dist/check/$name/zcrowd$ext"
  check_clean dist/check
  rm -rf dist/stage dist/check
  ls -l "dist/$archive"
}

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  main "$@"
fi
