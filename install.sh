#!/usr/bin/env bash
set -euo pipefail

# KaidaDB Installer
# Builds and installs kaidadb-server, kaidadb-cli, and kaidadb-tui binaries.

INSTALL_DIR="${KAIDADB_INSTALL_DIR:-$HOME/.local/bin}"
DATA_DIR="${KAIDADB_DATA_DIR:-$HOME/.local/share/kaidadb}"
CONFIG_DIR="${KAIDADB_CONFIG_DIR:-$HOME/.config/kaidadb}"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

info()  { printf "${BLUE}[info]${NC}  %s\n" "$*"; }
ok()    { printf "${GREEN}[ok]${NC}    %s\n" "$*"; }
warn()  { printf "${YELLOW}[warn]${NC}  %s\n" "$*"; }
err()   { printf "${RED}[error]${NC} %s\n" "$*" >&2; }

usage() {
    cat <<EOF
Usage: $0 [OPTIONS]

Install KaidaDB binaries and set up default configuration.

Options:
  --prefix DIR      Install binaries to DIR (default: ~/.local/bin)
  --data DIR        Set data directory (default: ~/.local/share/kaidadb)
  --config DIR      Set config directory (default: ~/.config/kaidadb)
  --release         Build in release mode (default)
  --debug           Build in debug mode
  --server-only     Only install kaidadb-server
  --cli-only        Only install kaidadb-cli
  --no-config       Skip config file generation
  --uninstall       Remove installed binaries and config
  -h, --help        Show this help
EOF
}

BUILD_PROFILE="release"
INSTALL_SERVER=true
INSTALL_CLI=true
INSTALL_TUI=true
GENERATE_CONFIG=true
UNINSTALL=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --prefix)       INSTALL_DIR="$2"; shift 2 ;;
        --data)         DATA_DIR="$2"; shift 2 ;;
        --config)       CONFIG_DIR="$2"; shift 2 ;;
        --release)      BUILD_PROFILE="release"; shift ;;
        --debug)        BUILD_PROFILE="debug"; shift ;;
        --server-only)  INSTALL_CLI=false; INSTALL_TUI=false; shift ;;
        --cli-only)     INSTALL_SERVER=false; INSTALL_TUI=false; shift ;;
        --no-config)    GENERATE_CONFIG=false; shift ;;
        --uninstall)    UNINSTALL=true; shift ;;
        -h|--help)      usage; exit 0 ;;
        *)              err "Unknown option: $1"; usage; exit 1 ;;
    esac
done

# ── Uninstall ──

if $UNINSTALL; then
    info "Uninstalling KaidaDB..."
    for bin in kaidadb-server kaidadb-cli kaidadb-tui; do
        if [[ -f "$INSTALL_DIR/$bin" ]]; then
            rm -f "$INSTALL_DIR/$bin"
            ok "Removed $INSTALL_DIR/$bin"
        fi
    done
    if [[ -f "$CONFIG_DIR/config.toml" ]]; then
        warn "Config left at $CONFIG_DIR/config.toml (remove manually if desired)"
    fi
    if [[ -d "$DATA_DIR" ]]; then
        warn "Data left at $DATA_DIR (remove manually if desired)"
    fi
    ok "Uninstall complete"
    exit 0
fi

# ── Preflight checks ──

info "Checking prerequisites..."

if ! command -v cargo &>/dev/null; then
    err "Rust toolchain not found. Install from https://rustup.rs/"
    exit 1
fi
ok "cargo $(cargo --version | awk '{print $2}')"

if ! command -v protoc &>/dev/null; then
    warn "protoc not found — gRPC codegen may fail"
    warn "Install: https://grpc.io/docs/protoc-installation/"
fi

# ── Resolve source directory ──

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ ! -f "$SCRIPT_DIR/Cargo.toml" ]]; then
    err "Run this script from the KaidaDB repository root"
    exit 1
fi
cd "$SCRIPT_DIR"

# ── Build ──

PACKAGES=()
$INSTALL_SERVER && PACKAGES+=("-p" "kaidadb-server")
$INSTALL_CLI    && PACKAGES+=("-p" "kaidadb-cli")
$INSTALL_TUI    && PACKAGES+=("-p" "kaidadb-tui")

if [[ ${#PACKAGES[@]} -eq 0 ]]; then
    err "Nothing to install"
    exit 1
fi

info "Building KaidaDB (${BUILD_PROFILE})..."
if [[ "$BUILD_PROFILE" == "release" ]]; then
    cargo build --release "${PACKAGES[@]}"
    BUILD_DIR="target/release"
else
    cargo build "${PACKAGES[@]}"
    BUILD_DIR="target/debug"
fi
ok "Build complete"

# ── Install binaries ──

mkdir -p "$INSTALL_DIR"

install_bin() {
    local name="$1"
    if [[ -f "$BUILD_DIR/$name" ]]; then
        cp "$BUILD_DIR/$name" "$INSTALL_DIR/$name"
        chmod +x "$INSTALL_DIR/$name"
        ok "Installed $INSTALL_DIR/$name"
    else
        warn "Binary $name not found in $BUILD_DIR"
    fi
}

info "Installing to $INSTALL_DIR..."
$INSTALL_SERVER && install_bin "kaidadb-server"
$INSTALL_CLI    && install_bin "kaidadb-cli"
$INSTALL_TUI    && install_bin "kaidadb-tui"

# ── Generate config ──

if $GENERATE_CONFIG; then
    mkdir -p "$CONFIG_DIR"
    mkdir -p "$DATA_DIR"

    CONFIG_FILE="$CONFIG_DIR/config.toml"
    if [[ -f "$CONFIG_FILE" ]]; then
        warn "Config already exists at $CONFIG_FILE (skipping)"
    else
        cat > "$CONFIG_FILE" <<TOML
# KaidaDB configuration
# See: https://github.com/your-repo/KaidaDB#configuration

data_dir = "$DATA_DIR"
grpc_addr = "0.0.0.0:50051"
rest_addr = "0.0.0.0:8080"

[storage]
chunk_size = 2097152  # 2 MiB

[cache]
max_size = 536870912  # 512 MiB
prefetch_window = 3
warm_on_write = false
TOML
        ok "Created config at $CONFIG_FILE"
    fi
fi

# ── Verify PATH ──

if ! echo "$PATH" | tr ':' '\n' | grep -qx "$INSTALL_DIR"; then
    warn "$INSTALL_DIR is not in your PATH"
    echo ""
    echo "  Add it to your shell config:"
    echo ""
    echo "    # bash/zsh:"
    echo "    echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> ~/.bashrc"
    echo ""
    echo "    # fish:"
    echo "    fish_add_path $INSTALL_DIR"
    echo ""
fi

# ── Summary ──

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
ok "KaidaDB installed successfully!"
echo ""
echo "  Binaries:  $INSTALL_DIR"
echo "  Config:    $CONFIG_DIR/config.toml"
echo "  Data:      $DATA_DIR"
echo ""
echo "  Quick start:"
echo ""
echo "    # Start the server"
echo "    kaidadb-server --config $CONFIG_DIR/config.toml"
echo ""
echo "    # Store a file"
echo "    kaidadb-cli store my-video ./video.mp4"
echo ""
echo "    # Launch the TUI"
echo "    kaidadb-tui"
echo ""
echo "    # Health check"
echo "    curl http://localhost:8080/v1/health"
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
