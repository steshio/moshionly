#!/usr/bin/env bash
#
# Builds Moshionly.app for Apple silicon Macs from a release build of the
# moshionly binary, with an ad-hoc signature.
#
#   scripts/release-mac.sh         build dist/Moshionly.app
#   scripts/release-mac.sh --help  every option

set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
repo_root=$(cd "$script_dir/.." && pwd -P)
readonly script_dir repo_root

readonly app_name="Moshionly"
readonly bin_name="moshionly"
readonly package="moshionly-app"
readonly plist_template="packaging/macos/Info.plist"
readonly app_id_source="crates/moshionly-app/src/app.rs"
# The oldest macOS the app is built for. macOS 11 was the first to run on
# Apple silicon, and what the binary links needs it too: Skia's prebuilt
# library is built for 11.0 and UniformTypeIdentifiers arrived in 11.
readonly minimum_macos="11.0"

out_dir="$repo_root/dist"
# Set as the script goes.
work_dir=""
problems=0
target_dir=""
version=""
build_number=""
copyright_year=""
binary=""
icon_tool=""
app=""

usage() {
  cat <<EOF
Usage: scripts/release-mac.sh [--out <folder>]

Builds Moshionly.app for Apple silicon Macs and puts it in dist/ at the
top of the repository. The app has an ad-hoc signature: it runs on this
Mac, but macOS warns people who download it.

  --out <folder>     Put the app in this folder instead of dist/.
  -h, --help         Show this text.

Settings, from the environment:
  CARGO_TARGET_DIR   Where cargo builds, as usual.
EOF
}

# One line for each step, so the person can follow along.
step() {
  printf '\n→ %s\n' "$1"
}

say() {
  printf '%s\n' "$@"
}

# Prints every line of every argument after the first, starting each with
# the first argument.
indented() {
  local prefix=$1 text line
  shift
  for text in "$@"; do
    while IFS= read -r line || [[ -n $line ]]; do
      if [[ -n $line ]]; then
        printf '%s%s\n' "$prefix" "$line"
      else
        printf '\n'
      fi
    done <<<"$text"
  done
}

# Stops with a message: the first argument says what went wrong, the others
# what to do about it.
die() {
  printf '\nStopped: %s\n' "$1" >&2
  shift
  indented "  " "$@" >&2
  exit 1
}

usage_error() {
  printf 'release-mac.sh: %s\nRun scripts/release-mac.sh --help for the options.\n' "$1" >&2
  exit 2
}

# Notes a problem found before building; every problem is reported before
# the script stops, so they can all be fixed in one go.
problem() {
  if [[ $problems -eq 0 ]]; then
    printf '\n%s\n' "$app_name.app cannot be built yet:" >&2
  fi
  problems=$((problems + 1))
  printf '\n  • %s\n' "$1" >&2
  shift
  indented "    " "$@" >&2
}

cleanup() {
  if [[ -n $work_dir && -d $work_dir ]]; then
    rm -rf "$work_dir"
  fi
}

absolute_path() {
  case $1 in
    /*) printf '%s\n' "$1" ;;
    *) printf '%s\n' "$PWD/$1" ;;
  esac
}

# A path as the person would like to read it: relative to the repository
# when it is inside it.
shown_path() {
  case $1 in
    "$repo_root"/*) printf '%s\n' "${1#"$repo_root"/}" ;;
    *) printf '%s\n' "$1" ;;
  esac
}

parse_args() {
  while [[ $# -gt 0 ]]; do
    case $1 in
      --out)
        if [[ $# -lt 2 || -z $2 ]]; then
          usage_error "--out needs a folder."
        fi
        out_dir=$(absolute_path "$2")
        shift
        ;;
      --out=*)
        if [[ -z ${1#--out=} ]]; then
          usage_error "--out needs a folder."
        fi
        out_dir=$(absolute_path "${1#--out=}")
        ;;
      -h | --help)
        usage
        exit 0
        ;;
      *) usage_error "unknown option: $1" ;;
    esac
    shift
  done
}

check_platform() {
  if [[ $(uname -s) != Darwin ]]; then
    die "this script builds the Mac app, so it runs only on macOS."
  fi
  if [[ $(uname -m) != arm64 ]]; then
    die "Moshionly.app is built for Apple silicon, and this terminal is not running on it (it reports $(uname -m))." \
      "Use an Apple silicon Mac, in a terminal that does not run under Rosetta."
  fi
}

# Runs a developer tool once to see that it works; prints what it said when
# it does not. Xcode's tools refuse to run until its licence is accepted.
tool_runs() {
  local output
  if output=$("$@" 2>&1 >/dev/null); then
    return 0
  fi
  if [[ $output == *"license"* ]]; then
    problem "Apple's developer tools do not run yet: Xcode's licence has not been accepted." \
      "Accept it once with:  sudo xcodebuild -license accept" \
      "(or open Xcode and agree), then run this again."
  else
    problem "\"$*\" does not run:" "${output:-it stopped without saying why.}"
  fi
  return 1
}

check_tools() {
  # rustup puts cargo here; a terminal opened before installing Rust may
  # not know yet.
  if ! command -v cargo >/dev/null 2>&1 && [[ -x $HOME/.cargo/bin/cargo ]]; then
    PATH="$HOME/.cargo/bin:$PATH"
  fi
  if ! command -v cargo >/dev/null 2>&1; then
    problem "Rust is not installed (cargo was not found)." "Install it from https://rustup.rs, then open a new terminal."
  fi
  # Everything else comes with macOS or with the Xcode command line tools,
  # which also provide the linker cargo needs.
  if ! xcode-select -p >/dev/null 2>&1; then
    problem "The Xcode command line tools are not installed." "Install them with:  xcode-select --install"
    return
  fi
  if ! tool_runs cc --version; then
    return
  fi
  local tool
  for tool in codesign ditto iconutil lipo otool plutil spctl vtool; do
    if ! command -v "$tool" >/dev/null 2>&1; then
      problem "$tool was not found. It comes with macOS and the Xcode command line tools."
    fi
  done
}

# Everything that can stop the build is checked here, before it starts.
preflight() {
  step "Checking the tools…"
  check_tools
  if [[ $problems -gt 0 ]]; then
    printf '\nNothing was built.\n' >&2
    exit 1
  fi
}

read_metadata() {
  local metadata="$work_dir/metadata.json"
  cargo metadata --format-version 1 --no-deps --locked >"$metadata"
  target_dir=$(plutil -extract target_directory raw -o - "$metadata")
  local count index
  count=$(plutil -extract packages raw -o - "$metadata")
  for ((index = 0; index < count; index++)); do
    if [[ $(plutil -extract "packages.$index.name" raw -o - "$metadata") == "$package" ]]; then
      version=$(plutil -extract "packages.$index.version" raw -o - "$metadata")
    fi
  done
  if [[ -z $version ]]; then
    die "cargo does not know the $package package." "Run this script from a Moshionly checkout."
  fi
  if ! [[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    die "the version in Cargo.toml, $version, is not one macOS accepts." \
      "An app's version is three numbers, such as 1.2.0."
  fi
  # The build number goes up with every commit, so two builds of one
  # version can be told apart. A shallow clone does not know the count.
  if git -C "$repo_root" rev-parse --verify --quiet HEAD >/dev/null 2>&1 &&
    [[ $(git -C "$repo_root" rev-parse --show-toplevel) == "$repo_root" ]]; then
    copyright_year=$(git -C "$repo_root" log -1 --format=%cd --date=format:%Y)
    if [[ $(git -C "$repo_root" rev-parse --is-shallow-repository) == false ]]; then
      build_number=$(git -C "$repo_root" rev-list --count HEAD)
    fi
  fi
  build_number=${build_number:-$version}
  copyright_year=${copyright_year:-$(date +%Y)}
}

build_binary() {
  step "Building the release binary (the first build takes several minutes)…"
  # Two cargo runs: built together, the example's test-only dependencies
  # could switch features on in the app's own binary.
  MACOSX_DEPLOYMENT_TARGET=$minimum_macos \
    cargo build --release --locked -p "$package" --bin "$bin_name"
  MACOSX_DEPLOYMENT_TARGET=$minimum_macos \
    cargo build --release --locked -p "$package" --example app_icon
  binary="$target_dir/release/$bin_name"
  icon_tool="$target_dir/release/examples/app_icon"
  if [[ ! -x $binary || ! -x $icon_tool ]]; then
    die "cargo finished, but $binary is not there." \
      "If CARGO_BUILD_TARGET or a cargo config changes where builds go, unset it and try again."
  fi
}

# The binary must be for Apple silicon, built for the oldest macOS the app
# promises, and link nothing but the system's own libraries: anything else
# would be missing on other people's Macs.
check_binary() {
  step "Checking the binary: Apple silicon, macOS $minimum_macos or later, system libraries only…"
  local archs minos libraries library foreign=""
  archs=$(lipo -archs "$binary")
  if [[ $archs != arm64 ]]; then
    die "the binary is built for \"$archs\", not for Apple silicon (arm64)."
  fi
  minos=$(vtool -show-build "$binary" | awk '$1 == "minos" { print $2; exit }')
  if [[ $minos != "$minimum_macos" ]]; then
    die "the binary says it needs macOS ${minos:-(unknown)}, but the app is for macOS $minimum_macos and later." \
      "An earlier build made with other settings may be in the way. Run  cargo clean --release  and try again."
  fi
  libraries=$(otool -L "$binary" | tail -n +2 | awk '{ print $1 }')
  while IFS= read -r library; do
    case $library in
      /usr/lib/* | /System/Library/*) ;;
      *) foreign="$foreign  $library"$'\n' ;;
    esac
  done <<<"$libraries"
  if [[ -n $foreign ]]; then
    die "the binary links libraries that other Macs do not have:" "$foreign" \
      "Moshionly.app may only use libraries that come with macOS."
  fi
}

make_icon() {
  step "Drawing the app icon…"
  "$icon_tool" "$work_dir/AppIcon.iconset" >/dev/null
  iconutil --convert icns --output "$work_dir/AppIcon.icns" "$work_dir/AppIcon.iconset"
}

write_info_plist() {
  local plist=$1 text placeholder='@[A-Z_]+@'
  text=$(<"$repo_root/$plist_template")
  text=${text//@VERSION@/$version}
  text=${text//@BUILD_NUMBER@/$build_number}
  text=${text//@MINIMUM_SYSTEM_VERSION@/$minimum_macos}
  text=${text//@COPYRIGHT_YEAR@/$copyright_year}
  if [[ $text =~ $placeholder ]]; then
    die "$plist_template has a placeholder this script does not fill in: ${BASH_REMATCH[0]}"
  fi
  printf '%s\n' "$text" >"$plist"
  if ! plutil -lint -s "$plist"; then
    die "the Info.plist made from $plist_template is not valid."
  fi
  # Rewritten by plutil, without the template's comment.
  plutil -convert xml1 "$plist"

  local bundle_id app_id executable
  bundle_id=$(plutil -extract CFBundleIdentifier raw -o - "$plist")
  executable=$(plutil -extract CFBundleExecutable raw -o - "$plist")
  app_id=$(sed -n 's/^pub const APP_ID: &str = "\(.*\)";$/\1/p' "$repo_root/$app_id_source")
  if [[ $bundle_id != "$app_id" ]]; then
    die "the bundle identifier in $plist_template ($bundle_id) differs from APP_ID in $app_id_source (${app_id:-not found})." \
      "macOS and the app must agree on it: make them the same."
  fi
  if [[ $executable != "$bin_name" ]]; then
    die "CFBundleExecutable in $plist_template should be $bin_name, not $executable."
  fi
}

assemble_bundle() {
  step "Assembling $app_name.app (version $version, build $build_number)…"
  app="$work_dir/build/$app_name.app"
  mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
  cp -X "$binary" "$app/Contents/MacOS/$bin_name"
  cp -X "$work_dir/AppIcon.icns" "$app/Contents/Resources/AppIcon.icns"
  write_info_plist "$app/Contents/Info.plist"
}

sign_app() {
  # Code inside a bundle is signed before the bundle. The only code here is
  # the main binary, which signing the bundle signs; anything else would
  # need signing first, so stop if something has appeared.
  local contents expected
  contents=$(cd "$app" && find . -type f -o -type l | LC_ALL=C sort)
  expected=$(printf '%s\n' ./Contents/Info.plist "./Contents/MacOS/$bin_name" ./Contents/Resources/AppIcon.icns)
  if [[ $contents != "$expected" ]]; then
    die "the app holds files this script does not know how to sign:" "$contents"
  fi

  # The hardened runtime is on, as a release needs it. No entitlements: the
  # app needs none.
  local -a args=(--force --options runtime --sign -)
  step "Signing the app (ad-hoc, with the hardened runtime)…"
  local output details
  if ! output=$(codesign "${args[@]}" "$app" 2>&1); then
    die "codesign could not sign the app:" "$output"
  fi
  if ! output=$(codesign --verify --strict --deep --verbose=2 "$app" 2>&1); then
    die "the signed app does not verify:" "$output"
  fi
  details=$(codesign -dvvv "$app" 2>&1)
  if [[ $details != *"flags="*"runtime"* ]]; then
    die "the app was signed without the hardened runtime."
  fi
}

# The results are copied out only once everything has worked, so a failed
# run never leaves half a release behind.
install_outputs() {
  step "Copying the results to $(shown_path "$out_dir")…"
  mkdir -p "$out_dir"
  local out_app="$out_dir/$app_name.app"
  rm -rf "$out_app"
  ditto "$app" "$out_app"
  app=$out_app
}

# Asks macOS what it will make of the app. Gatekeeper's verdict is what
# people who download it will meet.
final_checks() {
  step "Checking the app the way macOS will…"
  local output app_verdict
  if ! output=$(codesign --verify --strict --deep "$app" 2>&1); then
    die "the copied app does not verify:" "$output"
  fi
  app_verdict=$(spctl --assess --type execute --verbose=4 "$app" 2>&1) || true
  # spctl starts with the path, and says the rest over a few lines.
  app_verdict=${app_verdict//"$app": /}
  say "  Gatekeeper on the app: ${app_verdict//$'\n'/, }"
}

summary() {
  say "" "Done." \
    "  App:  $(shown_path "$app")" \
    "  Version $version (build $build_number), for Apple silicon Macs with macOS $minimum_macos or later."
  say "" "Ad-hoc signed. It runs on this Mac, but macOS will warn people who download it," \
    "because no Apple-issued certificate vouches for it: they would need" \
    "System Settings › Privacy & Security › Open Anyway."
}

main() {
  parse_args "$@"
  cd "$repo_root"
  check_platform
  preflight

  trap cleanup EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM
  local tmp_root="${TMPDIR:-/tmp}"
  work_dir=$(mktemp -d "${tmp_root%/}/moshionly-release.XXXXXX")

  read_metadata
  build_binary
  check_binary
  make_icon
  assemble_bundle
  sign_app
  install_outputs
  final_checks
  summary
}

main "$@"
