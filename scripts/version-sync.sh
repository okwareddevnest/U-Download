#!/bin/bash

# Dynamic Version Synchronization Script for U-Download
# This script ensures all version references are synchronized from a single source of truth

set -e  # Exit on any error

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Function to print colored messages
print_message() {
    local color=$1
    local message=$2
    echo -e "${color}${message}${NC}"
}

print_success() {
    print_message $GREEN "✅ $1"
}

print_error() {
    print_message $RED "❌ $1"
}

print_warning() {
    print_message $YELLOW "⚠️  $1"
}

print_info() {
    print_message $BLUE "ℹ️  $1"
}

# Function to show usage
show_usage() {
    echo "Dynamic Version Synchronization Script for U-Download"
    echo ""
    echo "Usage: $0 [VERSION] [OPTIONS]"
    echo ""
    echo "Arguments:"
    echo "  VERSION       Specific version to set (e.g., 2.3.0, 1.0.0-beta.1)"
    echo "                If not provided, version will be derived from Git"
    echo ""
    echo "Options:"
    echo "  --get         Just get the current version, don't update files"
    echo "  --dry-run     Show what would be updated without making changes"
    echo "  --help        Show this help message"
    echo ""
    echo "Examples:"
    echo "  $0                    # Auto-derive version from Git and update all files"
    echo "  $0 2.3.0              # Set specific version and update all files"
    echo "  $0 --get              # Just show current version"
    echo "  $0 2.3.0 --dry-run    # Preview changes without applying them"
    echo ""
    echo "Files that will be updated:"
    echo "  - package.json"
    echo "  - src-tauri/Cargo.toml"
    echo "  - src-tauri/tauri.conf.json"
    echo "  - packaging/homebrew/u-download.rb"
    echo "  - packaging/chocolatey/u-download.nuspec"
    echo "  - packaging/scoop/u-download.json"
    echo "  - And other packaging manifests"
}

# Check if Node.js is available
check_nodejs() {
    if ! command -v node &> /dev/null; then
        print_error "Node.js is not installed or not in PATH"
        print_info "Please install Node.js to use this script"
        exit 1
    fi
}

# Main function
main() {
    local version=""
    local get_only=false
    local dry_run=false

    # Parse arguments
    while [[ $# -gt 0 ]]; do
        case $1 in
            --help|-h)
                show_usage
                exit 0
                ;;
            --get)
                get_only=true
                shift
                ;;
            --dry-run)
                dry_run=true
                shift
                ;;
            -*)
                print_error "Unknown option: $1"
                show_usage
                exit 1
                ;;
            *)
                if [[ -z "$version" ]]; then
                    version="$1"
                else
                    print_error "Multiple version arguments provided"
                    show_usage
                    exit 1
                fi
                shift
                ;;
        esac
    done

    # Check prerequisites
    check_nodejs

    cd "$PROJECT_ROOT"

    # Ensure we're in a git repository
    if ! git rev-parse --git-dir > /dev/null 2>&1; then
        print_warning "Not in a Git repository, using fallback version detection"
    fi

    print_info "U-Download Dynamic Version Synchronization"
    print_info "Project root: $PROJECT_ROOT"
    echo ""

    # Build the command for the Node.js script
    local node_cmd="node '$SCRIPT_DIR/version-from-git.cjs'"
    
    if [[ "$get_only" == "true" ]]; then
        node_cmd="$node_cmd --get"
    else
        if [[ -n "$version" ]]; then
            node_cmd="$node_cmd --version '$version'"
        fi
        node_cmd="$node_cmd --update"
    fi

    if [[ "$dry_run" == "true" ]]; then
        print_info "DRY RUN MODE - No files will be modified"
        print_info "Command that would be executed:"
        print_info "$node_cmd"
        echo ""
    fi

    # Execute the Node.js script
    if [[ "$dry_run" == "true" ]]; then
        # For dry run, we still want to show what version would be used
        local detected_version
        if [[ -n "$version" ]]; then
            detected_version="$version"
        else
            detected_version=$(node "$SCRIPT_DIR/version-from-git.cjs" --get)
        fi
        print_info "Would synchronize to version: $detected_version"
        
        # List files that would be updated
        print_info "Files that would be updated:"
        echo "  - package.json"
        echo "  - src-tauri/Cargo.toml"
        echo "  - src-tauri/tauri.conf.json"
        echo "  - packaging/homebrew/u-download.rb"
        echo "  - packaging/chocolatey/u-download.nuspec"
        echo "  - packaging/scoop/u-download.json"
        echo ""
        print_success "Dry run completed successfully"
    else
        # Execute the actual command
        eval "$node_cmd"
        
        if [[ $? -eq 0 ]]; then
            if [[ "$get_only" != "true" ]]; then
                echo ""
                print_success "Version synchronization completed successfully!"
                print_info "All version references have been updated"
                echo ""
                print_info "Next steps:"
                print_info "1. Review the changes: git diff"
                print_info "2. Test the build: npm run tauri:build"
                print_info "3. Commit the changes: git add . && git commit -m 'chore: sync version'"
            fi
        else
            print_error "Version synchronization failed"
            exit 1
        fi
    fi
}

# Run main function with all arguments
main "$@"
