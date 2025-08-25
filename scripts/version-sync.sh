#!/bin/bash

# Script to sync version across package.json, Cargo.toml, and tauri.conf.json
# Usage: ./scripts/version-sync.sh [version]

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"

if [ -z "$1" ]; then
    echo "Usage: $0 <version>"
    echo "Example: $0 1.0.0"
    exit 1
fi

NEW_VERSION="$1"

# Validate semantic versioning format
if ! echo "$NEW_VERSION" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$'; then
    echo "Error: '$NEW_VERSION' is not a valid semantic version"
    echo "Format should be: MAJOR.MINOR.PATCH or MAJOR.MINOR.PATCH-PRERELEASE or MAJOR.MINOR.PATCH+BUILDMETADATA"
    exit 1
fi

echo "Updating version to $NEW_VERSION..."

# Update package.json
if command -v npm >/dev/null 2>&1; then
    cd "$ROOT_DIR"
    npm version "$NEW_VERSION" --no-git-tag-version
    echo "✓ Updated package.json"
else
    echo "Warning: npm not found, skipping package.json update"
fi

# Update Cargo.toml
CARGO_TOML="$ROOT_DIR/src-tauri/Cargo.toml"
if [ -f "$CARGO_TOML" ]; then
    if command -v cargo >/dev/null 2>&1; then
        cd "$ROOT_DIR/src-tauri"
        cargo set-version "$NEW_VERSION"
        echo "✓ Updated Cargo.toml"
    else
        # Fallback to sed if cargo-edit is not available
        sed -i.bak "s/^version = \".*\"/version = \"$NEW_VERSION\"/" "$CARGO_TOML"
        rm -f "$CARGO_TOML.bak"
        echo "✓ Updated Cargo.toml (using sed)"
    fi
else
    echo "Warning: Cargo.toml not found"
fi

# Update tauri.conf.json
TAURI_CONF="$ROOT_DIR/src-tauri/tauri.conf.json"
if [ -f "$TAURI_CONF" ]; then
    if command -v jq >/dev/null 2>&1; then
        jq ".version = \"$NEW_VERSION\"" "$TAURI_CONF" > "$TAURI_CONF.tmp" && mv "$TAURI_CONF.tmp" "$TAURI_CONF"
        echo "✓ Updated tauri.conf.json (using jq)"
    else
        # Fallback to sed
        sed -i.bak "s/\"version\": \".*\"/\"version\": \"$NEW_VERSION\"/" "$TAURI_CONF"
        rm -f "$TAURI_CONF.bak"
        echo "✓ Updated tauri.conf.json (using sed)"
    fi
else
    echo "Warning: tauri.conf.json not found"
fi

echo ""
echo "Version update complete! 🎉"
echo "New version: $NEW_VERSION"
echo ""
echo "To create a release, run:"
echo "  git add ."
echo "  git commit -m \"chore: bump version to $NEW_VERSION\""
echo "  git tag -a \"v$NEW_VERSION\" -m \"Release v$NEW_VERSION\""
echo "  git push origin main && git push origin \"v$NEW_VERSION\""