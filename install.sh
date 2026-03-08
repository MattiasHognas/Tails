#!/bin/bash
# Installer script for rag-cli
# This script detects the OS/architecture and installs or updates the rag-cli binary

set -e

# Configuration
REPO="MattiasHognas/tails"
BINARY_NAME="rag-cli"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"
VERSION="${VERSION:-latest}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Helper functions
log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Detect OS and architecture
detect_platform() {
    local os="$(uname -s)"
    local arch="$(uname -m)"
    
    case "$os" in
        Linux*)
            case "$arch" in
                x86_64)
                    PLATFORM="linux-x86_64"
                    ;;
                aarch64|arm64)
                    PLATFORM="linux-aarch64"
                    ;;
                *)
                    log_error "Unsupported architecture: $arch"
                    exit 1
                    ;;
            esac
            ;;
        Darwin*)
            case "$arch" in
                x86_64)
                    PLATFORM="macos-x86_64"
                    ;;
                arm64)
                    PLATFORM="macos-aarch64"
                    ;;
                *)
                    log_error "Unsupported architecture: $arch"
                    exit 1
                    ;;
            esac
            ;;
        *)
            log_error "Unsupported operating system: $os"
            exit 1
            ;;
    esac
    
    ASSET_NAME="${BINARY_NAME}-${PLATFORM}"
    log_info "Detected platform: $PLATFORM"
}

# Get the latest release or specific version
get_download_url() {
    if [ "$VERSION" = "latest" ]; then
        log_info "Fetching latest release information..."
        RELEASE_URL="https://api.github.com/repos/$REPO/releases/latest"
    else
        log_info "Fetching release information for version $VERSION..."
        RELEASE_URL="https://api.github.com/repos/$REPO/releases/tags/$VERSION"
    fi
    
    # Try to get release info from GitHub API
    RELEASE_JSON=$(curl -s "$RELEASE_URL")
    
    # Check if we got a valid response
    if echo "$RELEASE_JSON" | grep -q "Not Found"; then
        log_error "Release not found. Please check the version or repository."
        exit 1
    fi
    
    # Check for jq
    if ! command -v jq >/dev/null 2>&1; then
        log_error "jq is required but not installed. Please install jq and try again."
        exit 1
    fi
    
    # Extract download URL for the specific asset using jq
    DOWNLOAD_URL=$(echo "$RELEASE_JSON" | jq -r --arg NAME "$ASSET_NAME" '.assets[] | select(.name == $NAME) | .browser_download_url' | head -n 1)
    
    if [ -z "$DOWNLOAD_URL" ] || [ "$DOWNLOAD_URL" = "null" ]; then
        log_error "Could not find download URL for $ASSET_NAME"
        log_info "Available assets:"
        echo "$RELEASE_JSON" | jq -r '.assets[].browser_download_url'
        exit 1
    fi
    
    log_info "Download URL: $DOWNLOAD_URL"
}

# Check if binary is already installed
check_existing_installation() {
    if [ -f "$INSTALL_DIR/$BINARY_NAME" ]; then
        log_info "Existing installation found at $INSTALL_DIR/$BINARY_NAME"
        
        # Try to get version of installed binary
        if [ -x "$INSTALL_DIR/$BINARY_NAME" ]; then
            INSTALLED_VERSION=$("$INSTALL_DIR/$BINARY_NAME" --version 2>/dev/null || echo "unknown")
            log_info "Installed version: $INSTALLED_VERSION"
        fi
        
        return 0
    else
        log_info "No existing installation found"
        return 1
    fi
}

# Download and install the binary
install_binary() {
    local tmp_file="/tmp/${BINARY_NAME}-${PLATFORM}.tmp"
    local curl_err="/tmp/${BINARY_NAME}-${PLATFORM}.curl.err"
    
    log_info "Downloading $ASSET_NAME..."
    if ! curl -L -o "$tmp_file" "$DOWNLOAD_URL" 2>"$curl_err"; then
        log_error "Failed to download binary. curl output:"
        cat "$curl_err" >&2
        rm -f "$curl_err"
        exit 1
    fi
    rm -f "$curl_err"
    
    # Create install directory if it doesn't exist
    if [ ! -d "$INSTALL_DIR" ]; then
        log_info "Creating installation directory: $INSTALL_DIR"
        mkdir -p "$INSTALL_DIR"
    fi
    
    # Backup existing installation if it exists
    if [ -f "$INSTALL_DIR/$BINARY_NAME" ]; then
        log_info "Backing up existing installation..."
        mv "$INSTALL_DIR/$BINARY_NAME" "$INSTALL_DIR/$BINARY_NAME.backup"
    fi
    
    # Move the binary to installation directory
    log_info "Installing to $INSTALL_DIR/$BINARY_NAME..."
    mv "$tmp_file" "$INSTALL_DIR/$BINARY_NAME"
    chmod +x "$INSTALL_DIR/$BINARY_NAME"
    
    # Verify installation
    if [ -x "$INSTALL_DIR/$BINARY_NAME" ]; then
        NEW_VERSION=$("$INSTALL_DIR/$BINARY_NAME" --version 2>/dev/null || echo "installed")
        log_info "Installation successful! Version: $NEW_VERSION"
        
        # Remove backup if installation was successful
        if [ -f "$INSTALL_DIR/$BINARY_NAME.backup" ]; then
            rm "$INSTALL_DIR/$BINARY_NAME.backup"
        fi
    else
        log_error "Installation verification failed"
        
        # Restore backup if it exists
        if [ -f "$INSTALL_DIR/$BINARY_NAME.backup" ]; then
            log_warn "Restoring backup..."
            mv "$INSTALL_DIR/$BINARY_NAME.backup" "$INSTALL_DIR/$BINARY_NAME"
        fi
        exit 1
    fi
}

# Check if install directory is in PATH
check_path() {
    if ! echo "$PATH" | grep -q "$INSTALL_DIR"; then
        log_warn "Installation directory $INSTALL_DIR is not in your PATH"
        log_info "Add the following line to your shell profile (~/.bashrc, ~/.zshrc, etc.):"
        echo ""
        echo "    export PATH=\"$INSTALL_DIR:\$PATH\""
        echo ""
    else
        log_info "Installation directory is in PATH"
    fi
}

# Main installation flow
main() {
    log_info "Starting rag-cli installation..."
    
    detect_platform
    
    if check_existing_installation; then
        log_info "Updating existing installation..."
    else
        log_info "Performing fresh installation..."
    fi
    
    get_download_url
    install_binary
    check_path
    
    log_info "Installation complete!"
    log_info "Run '$BINARY_NAME --help' to get started"
}

# Run main function
main
