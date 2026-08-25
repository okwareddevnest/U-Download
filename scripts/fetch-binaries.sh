#!/usr/bin/env bash
#
# fetch-binaries.sh — download yt-dlp, ffmpeg and aria2c into
# src-tauri/binaries/<platform>/ so the Tauri build can bundle them.
#
# Binaries are no longer stored in Git (the LFS budget for this repo is
# exhausted); they are fetched from upstream release infrastructure at
# build time instead. See binary_manager.rs (platform_dir()/exe_name())
# for the directory names and filenames this script must produce.
#
# Usage:
#   scripts/fetch-binaries.sh [platform] [--force]
#
#   platform  Optional override: linux-x64 | linux-arm64 | macos-x64 |
#             macos-arm64 | windows-x64. Defaults to the host platform,
#             detected via `uname`. Pass this explicitly in CI when
#             cross-building (e.g. building linux-arm64 on an x86_64 host).
#   --force   Re-download even if a valid binary is already present.
#
# The script is idempotent: if a binary already exists, is executable,
# and is larger than a sane minimum size (i.e. it isn't a 3-line Git LFS
# pointer stub), it is left alone unless --force is given.
#
# It fails loudly: every download is checked for HTTP success, sanity
# checked to make sure it isn't an HTML error page, and — after
# extraction — verified to have the executable magic bytes appropriate
# for the target platform before being installed. A build must never
# silently ship a 404 page named "yt-dlp".

set -euo pipefail

# Force byte-safe behavior for head/tr/od on arbitrary binary content,
# regardless of the invoking shell's locale (avoids "tr: Illegal byte
# sequence" when a UTF-8 locale chokes on raw binary bytes).
export LC_ALL=C

# ---------------------------------------------------------------------------
# Setup
# ---------------------------------------------------------------------------

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

MIN_BINARY_SIZE=200000  # bytes; real binaries are multi-MB, LFS stubs are ~130 bytes
CURL_RETRY_ARGS=(--retry 3 --retry-delay 2 --connect-timeout 15 --max-time 600)

FETCHED=()
SKIPPED=()

log()  { printf '%s\n' "$*" >&2; }
fail() { log "ERROR: $*"; exit 1; }

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------

FORCE=0
PLATFORM_OVERRIDE=""

for arg in "$@"; do
  case "$arg" in
    --force)
      FORCE=1
      ;;
    -h|--help)
      sed -n '2,29p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    linux-x64|linux-arm64|macos-x64|macos-arm64|windows-x64)
      PLATFORM_OVERRIDE="$arg"
      ;;
    *)
      fail "Unrecognized argument: '$arg'. Expected one of linux-x64, linux-arm64, macos-x64, macos-arm64, windows-x64, or --force."
      ;;
  esac
done

# ---------------------------------------------------------------------------
# Platform detection (must match binary_manager.rs::platform_dir())
# ---------------------------------------------------------------------------

detect_platform() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"

  case "$os" in
    Linux)
      case "$arch" in
        x86_64) echo "linux-x64" ;;
        aarch64|arm64) echo "linux-arm64" ;;
        *) fail "Unsupported Linux architecture: $arch" ;;
      esac
      ;;
    Darwin)
      case "$arch" in
        x86_64) echo "macos-x64" ;;
        arm64) echo "macos-arm64" ;;
        *) fail "Unsupported macOS architecture: $arch" ;;
      esac
      ;;
    MINGW*|MSYS*|CYGWIN*|Windows_NT)
      # Git Bash / MSYS on GitHub Actions windows-latest runners are x86_64 only today.
      echo "windows-x64"
      ;;
    *)
      fail "Unsupported OS from uname -s: '$os'. Pass a platform explicitly, e.g. scripts/fetch-binaries.sh linux-x64"
      ;;
  esac
}

if [ -n "$PLATFORM_OVERRIDE" ]; then
  PLATFORM="$PLATFORM_OVERRIDE"
else
  PLATFORM="$(detect_platform)"
fi

case "$PLATFORM" in
  linux-x64|linux-arm64|macos-x64|macos-arm64|windows-x64) ;;
  *) fail "Internal error: unrecognized platform '$PLATFORM'" ;;
esac

BIN_DIR="$REPO_ROOT/src-tauri/binaries/$PLATFORM"
mkdir -p "$BIN_DIR"

EXE_SUFFIX=""
if [ "$PLATFORM" = "windows-x64" ]; then
  EXE_SUFFIX=".exe"
fi

log "==> Target platform: $PLATFORM"
log "==> Destination:     $BIN_DIR"

# ---------------------------------------------------------------------------
# Download / verification helpers
# ---------------------------------------------------------------------------

# Downloads $1 (URL) to $2 (dest file). Fails loudly on HTTP error or an
# HTML error page masquerading as a 200 response.
download() {
  local url="$1" dest="$2"
  log "    fetching: $url"
  if ! curl -fsSL "${CURL_RETRY_ARGS[@]}" -o "$dest" "$url"; then
    rm -f "$dest"
    fail "Download failed for $url"
  fi

  if [ ! -s "$dest" ]; then
    fail "Downloaded file is empty: $url"
  fi

  # Common silent-failure mode: a redirect/CDN returns 200 with an HTML
  # error/landing page instead of the binary asset.
  local head
  head="$(head -c 64 "$dest" 2>/dev/null | tr -d '\0')"
  case "$head" in
    "<"*|*"<html"*|*"<HTML"*|*"<!DOCTYPE"*)
      rm -f "$dest"
      fail "Downloaded content from $url looks like an HTML page, not a binary/archive. Aborting."
      ;;
  esac
}

# Extracts archive $1 into directory $2, auto-detecting zip vs tar.*.
extract_archive() {
  local archive="$1" outdir="$2"
  mkdir -p "$outdir"
  case "$archive" in
    *.zip)
      if command -v unzip >/dev/null 2>&1; then
        unzip -oq "$archive" -d "$outdir"
      else
        # bsdtar (present on macOS and Git-for-Windows) can also extract zip.
        tar -xf "$archive" -C "$outdir"
      fi
      ;;
    *.tar.xz|*.tar.gz|*.tgz|*.tar.bz2)
      tar -xf "$archive" -C "$outdir"
      ;;
    *)
      fail "Don't know how to extract archive: $archive"
      ;;
  esac
}

# Finds a file named exactly $2 somewhere under directory $1.
find_in_extracted() {
  local dir="$1" name="$2" found
  found="$(find "$dir" -type f -name "$name" 2>/dev/null | head -n 1)"
  if [ -z "$found" ]; then
    fail "Could not find '$name' inside extracted archive under $dir"
  fi
  echo "$found"
}

# Soft check: does the file at $1 look like a plausible executable for
# $PLATFORM (right size, right magic bytes)? Returns 0/1, never exits —
# safe to call from a conditional (e.g. the idempotency skip path), unlike
# verify_binary() below. Logs the specific reason on failure.
binary_looks_valid() {
  local f="$1"
  if [ ! -f "$f" ]; then
    log "    check failed: $f does not exist"
    return 1
  fi

  local size
  size="$(wc -c < "$f" | tr -d ' ')"
  if [ "$size" -lt "$MIN_BINARY_SIZE" ]; then
    log "    check failed: $f is only $size bytes — too small to be a real binary (expected a Git LFS pointer stub or a truncated/error download)."
    return 1
  fi

  local magic
  magic="$(head -c 4 "$f" | od -An -tx1 2>/dev/null | tr -d ' \n')"
  case "$PLATFORM" in
    linux-*)
      if [ "$magic" != "7f454c46" ]; then
        log "    check failed: $f does not have an ELF header (magic=$magic)."
        return 1
      fi
      ;;
    macos-*)
      case "$magic" in
        cffaedfe|feedfacf|feedface|cefaedfe|cafebabe|bebafeca) : ;;
        *)
          log "    check failed: $f does not have a Mach-O header (magic=$magic)."
          return 1
          ;;
      esac
      ;;
    windows-*)
      if [ "${magic:0:4}" != "4d5a" ]; then
        log "    check failed: $f does not have an MZ/PE header (magic=$magic)."
        return 1
      fi
      ;;
  esac

  return 0
}

# Verifies the file at $1 is a plausible executable for $PLATFORM. Fails
# the whole script loudly if not — used right after a fresh download, where
# there is no sane fallback other than aborting.
verify_binary() {
  local f="$1"
  if ! binary_looks_valid "$f"; then
    fail "File $f failed integrity checks (size/magic bytes). Refusing to install/trust it."
  fi
}

# Computes a checksum of $2 using algorithm $1 (sha256|md5), preferring
# GNU coreutils tools and falling back to the BSD/macOS equivalents.
compute_hash() {
  local algo="$1" file="$2"
  case "$algo" in
    sha256)
      if command -v sha256sum >/dev/null 2>&1; then
        local out
        out="$(sha256sum "$file")"
        echo "${out%% *}"
      elif command -v shasum >/dev/null 2>&1; then
        local out
        out="$(shasum -a 256 "$file")"
        echo "${out%% *}"
      else
        fail "Neither sha256sum nor shasum is available; cannot verify checksum for $file"
      fi
      ;;
    md5)
      if command -v md5sum >/dev/null 2>&1; then
        local out
        out="$(md5sum "$file")"
        echo "${out%% *}"
      elif command -v md5 >/dev/null 2>&1; then
        md5 -q "$file"
      else
        fail "Neither md5sum nor md5 is available; cannot verify checksum for $file"
      fi
      ;;
    *)
      fail "Unknown checksum algorithm: $algo"
      ;;
  esac
}

# Returns "algo:checksums_url" for the given tool on $PLATFORM, or an empty
# string if the upstream vendor does not publish a checksum for this asset.
# Checked against each vendor's live release at the time this script was
# written — see docs/superpowers/notes for the verification log.
checksum_source() {
  local tool="$1"
  case "$tool" in
    yt-dlp)
      # yt-dlp publishes a SHA2-256SUMS file covering every release asset.
      echo "sha256:https://github.com/yt-dlp/yt-dlp/releases/latest/download/SHA2-256SUMS"
      ;;
    ffmpeg)
      case "$PLATFORM" in
        linux-x64)
          echo "md5:https://johnvansickle.com/ffmpeg/releases/ffmpeg-release-amd64-static.tar.xz.md5" ;;
        linux-arm64)
          echo "md5:https://johnvansickle.com/ffmpeg/releases/ffmpeg-release-arm64-static.tar.xz.md5" ;;
        windows-x64)
          echo "sha256:https://github.com/BtbN/FFmpeg-Builds/releases/latest/download/checksums.sha256" ;;
        *)
          # macos-x64/macos-arm64: ffmpeg.martin-riedl.de does not publish
          # a checksum file for its builds (confirmed 404 on the obvious
          # /checksum and .sha256 endpoints).
          echo ""
          ;;
      esac
      ;;
    aria2c)
      # None of abcfy2/aria2-static-build (linux, windows) or
      # q741451/aria2c-macos-standalone-binary (macos) publish a checksum
      # file alongside their release assets. Confirmed by inspecting their
      # release asset lists — no .sha256/.md5/checksums file present.
      echo ""
      ;;
    *)
      echo ""
      ;;
  esac
}

# Verifies $3 (a local file just downloaded from $2) against the checksum
# published for $1 (tool name), if any. No-op (with a log line) if the
# vendor publishes no checksum for this source. Fails loudly on a mismatch
# or if the checksums file doesn't list this asset at all.
verify_checksum_for() {
  local tool="$1" url="$2" local_file="$3"
  local spec
  spec="$(checksum_source "$tool")"
  if [ -z "$spec" ]; then
    log "    (no published checksum available for $tool on $PLATFORM — relying on HTTPS + size/magic-byte checks only)"
    return
  fi

  local algo="${spec%%:*}"
  local sums_url="${spec#*:}"
  local match_name
  match_name="$(basename "$url")"

  local sums_file="$TMP_ROOT/sums-${tool}-$$"
  download "$sums_url" "$sums_file"

  local match_name_re="${match_name//./\.}"
  local expected
  # The `|| true` neutralizes grep's exit status when nothing matches (a
  # legitimate, expected outcome here — handled by the -z check below) so
  # that `set -e`/`pipefail` don't abort this bare assignment before the
  # check runs and can print a proper fail() message.
  expected="$(grep -E "[[:space:]]\*?${match_name_re}\$" "$sums_file" | head -n 1 | awk '{print $1}' || true)"
  if [ -z "$expected" ]; then
    fail "Could not find a checksum entry for '$match_name' in $sums_url — refusing to trust an unverifiable download."
  fi

  local actual
  actual="$(compute_hash "$algo" "$local_file")"
  if [ "$expected" != "$actual" ]; then
    fail "Checksum mismatch for $tool asset '$match_name': expected $algo $expected, got $actual. Refusing to install a corrupted or tampered download."
  fi

  log "    checksum OK ($algo): $match_name"
}

# Installs $1 (verified source file) as $2 (final dest in BIN_DIR), chmod +x.
install_binary() {
  local src="$1" dest="$2"
  mkdir -p "$(dirname "$dest")"
  cp "$src" "$dest"
  chmod +x "$dest" 2>/dev/null || true
}

TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/udl-fetch-bin.XXXXXX")"
cleanup() { rm -rf "$TMP_ROOT"; }
trap cleanup EXIT

# ---------------------------------------------------------------------------
# Source definitions
#
# Every URL below is a "latest" style alias that GitHub (or the vendor)
# keeps stable across releases, so this script does not need to be
# updated every time yt-dlp/ffmpeg/aria2 cut a new version. Asset names
# were verified against the live releases pages at the time this script
# was written — see docs/superpowers/notes for the verification log.
# ---------------------------------------------------------------------------

yt_dlp_url() {
  case "$PLATFORM" in
    linux-x64)   echo "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux" ;;
    linux-arm64) echo "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux_aarch64" ;;
    macos-x64|macos-arm64)
                 echo "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_macos" ;;
    windows-x64) echo "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe" ;;
  esac
}

ffmpeg_url() {
  case "$PLATFORM" in
    linux-x64)   echo "https://johnvansickle.com/ffmpeg/releases/ffmpeg-release-amd64-static.tar.xz" ;;
    linux-arm64) echo "https://johnvansickle.com/ffmpeg/releases/ffmpeg-release-arm64-static.tar.xz" ;;
    macos-x64)   echo "https://ffmpeg.martin-riedl.de/redirect/latest/macos/amd64/release/ffmpeg.zip" ;;
    macos-arm64) echo "https://ffmpeg.martin-riedl.de/redirect/latest/macos/arm64/release/ffmpeg.zip" ;;
    windows-x64) echo "https://github.com/BtbN/FFmpeg-Builds/releases/latest/download/ffmpeg-master-latest-win64-gpl.zip" ;;
  esac
}

aria2c_url() {
  case "$PLATFORM" in
    linux-x64)   echo "https://github.com/abcfy2/aria2-static-build/releases/latest/download/aria2-x86_64-linux-musl_static.zip" ;;
    linux-arm64) echo "https://github.com/abcfy2/aria2-static-build/releases/latest/download/aria2-aarch64-linux-musl_static.zip" ;;
    macos-x64)   echo "https://github.com/q741451/aria2c-macos-standalone-binary/releases/latest/download/aria2c-macos-x86_64.tar.gz" ;;
    macos-arm64) echo "https://github.com/q741451/aria2c-macos-standalone-binary/releases/latest/download/aria2c-macos-arm64.tar.gz" ;;
    windows-x64) echo "https://github.com/abcfy2/aria2-static-build/releases/latest/download/aria2-x86_64-w64-mingw32_static.zip" ;;
  esac
}

# ---------------------------------------------------------------------------
# Fetch one tool
#
# $1 = tool name (yt-dlp | ffmpeg | aria2c)
# $2 = source URL
# $3 = "raw" if the URL is the binary itself, "archive" if it needs extracting
# $4 = inner filename to locate inside the archive (only used when $3=archive)
# ---------------------------------------------------------------------------

fetch_tool() {
  local tool="$1" url="$2" kind="$3" inner_name="${4:-}"
  local dest="$BIN_DIR/${tool}${EXE_SUFFIX}"

  if [ -z "$url" ]; then
    fail "No source URL defined for $tool on platform $PLATFORM"
  fi

  if [ "$FORCE" -ne 1 ] && [ -f "$dest" ]; then
    chmod +x "$dest" 2>/dev/null || true
    if binary_looks_valid "$dest"; then
      local existing_size
      existing_size="$(wc -c < "$dest" | tr -d ' ')"
      log "==> $tool: already present and verified valid ($existing_size bytes), skipping (use --force to re-fetch)"
      SKIPPED+=("$tool|$dest|$existing_size bytes")
      return
    else
      log "==> $tool: existing file at $dest failed validity checks (see above) — refetching"
    fi
  fi

  log "==> $tool: fetching for $PLATFORM"

  local work_dir="$TMP_ROOT/$tool"
  mkdir -p "$work_dir"

  local resolved_binary
  if [ "$kind" = "raw" ]; then
    local raw_dest="$work_dir/${tool}${EXE_SUFFIX}"
    download "$url" "$raw_dest"
    verify_checksum_for "$tool" "$url" "$raw_dest"
    resolved_binary="$raw_dest"
  elif [ "$kind" = "archive" ]; then
    local archive_dest="$work_dir/archive-$(basename "$url")"
    download "$url" "$archive_dest"
    verify_checksum_for "$tool" "$url" "$archive_dest"
    local extract_dir="$work_dir/extracted"
    extract_archive "$archive_dest" "$extract_dir"
    resolved_binary="$(find_in_extracted "$extract_dir" "$inner_name")"
  else
    fail "Internal error: unknown kind '$kind' for $tool"
  fi

  verify_binary "$resolved_binary"
  install_binary "$resolved_binary" "$dest"
  verify_binary "$dest"

  local final_size
  final_size="$(wc -c < "$dest" | tr -d ' ')"
  log "==> $tool: installed at $dest ($final_size bytes)"
  FETCHED+=("$tool|$dest|$final_size bytes")
}

# ---------------------------------------------------------------------------
# Run
# ---------------------------------------------------------------------------

fetch_tool "yt-dlp" "$(yt_dlp_url)" "raw"
fetch_tool "ffmpeg" "$(ffmpeg_url)" "archive" "ffmpeg${EXE_SUFFIX}"
fetch_tool "aria2c" "$(aria2c_url)" "archive" "aria2c${EXE_SUFFIX}"

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------

log ""
log "================ fetch-binaries summary ================"
log "Platform:    $PLATFORM"
log "Destination: $BIN_DIR"
log ""
if [ "${#FETCHED[@]}" -gt 0 ]; then
  log "Fetched:"
  for entry in "${FETCHED[@]}"; do
    IFS='|' read -r name path size <<< "$entry"
    log "  - $name -> $path ($size)"
  done
fi
if [ "${#SKIPPED[@]}" -gt 0 ]; then
  log "Skipped (already present):"
  for entry in "${SKIPPED[@]}"; do
    IFS='|' read -r name path size <<< "$entry"
    log "  - $name -> $path ($size)"
  done
fi
log "=========================================================="
