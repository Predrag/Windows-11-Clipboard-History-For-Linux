#!/bin/bash
# release.sh - Automated release script
# Usage: ./scripts/release.sh 0.5.0
#    or: make release VERSION=0.5.0

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log()     { echo -e "${BLUE}[*]${NC} $1"; }
success() { echo -e "${GREEN}[✓]${NC} $1"; }
warn()    { echo -e "${YELLOW}[!]${NC} $1"; }
error()   { echo -e "${RED}[✗]${NC} $1"; exit 1; }

# Get version from argument
VERSION="${1:-}"

if [ -z "$VERSION" ]; then
    echo "Usage: $0 <version>"
    echo "Example: $0 0.5.0"
    echo ""
    echo "Current versions:"
    echo "  package.json:      $(grep '"version"' package.json | head -1 | sed 's/.*: "\(.*\)".*/\1/')"
    echo "  tauri.conf.json:   $(grep '"version"' src-tauri/tauri.conf.json | head -1 | sed 's/.*: "\(.*\)".*/\1/')"
    echo "  Cargo.toml:        $(grep '^version' src-tauri/Cargo.toml | head -1 | sed 's/.*= "\(.*\)"/\1/')"
    exit 1
fi

# Validate version format (semver)
if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$ ]]; then
    error "Invalid version format: $VERSION (expected: X.Y.Z or X.Y.Z-suffix)"
fi

# Check for uncommitted changes
if [ -n "$(git status --porcelain)" ]; then
    warn "You have uncommitted changes:"
    git status --short
    echo ""
    read -p "Commit these changes before release? [Y/n] " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Nn]$ ]]; then
        git add -A
        git commit -m "chore: prepare for v$VERSION release"
        success "Changes committed"
    else
        error "Please commit or stash changes before releasing"
    fi
fi

# Check we're on main/master branch
BRANCH=$(git rev-parse --abbrev-ref HEAD)
if [[ "$BRANCH" != "main" && "$BRANCH" != "master" ]]; then
    warn "You're on branch '$BRANCH', not main/master"
    read -p "Continue anyway? [y/N] " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        exit 1
    fi
fi

log "Releasing version $VERSION..."

# Update package.json
log "Updating package.json..."
npm version "$VERSION" --no-git-tag-version --allow-same-version
success "package.json updated"

# Update tauri.conf.json
log "Updating tauri.conf.json..."
sed -i 's/"version": "[^"]*"/"version": "'"$VERSION"'"/' src-tauri/tauri.conf.json
success "tauri.conf.json updated"

# Update Cargo.toml (first occurrence only)
log "Updating Cargo.toml..."
sed -i '0,/^version = "[^"]*"/s//version = "'"$VERSION"'"/' src-tauri/Cargo.toml
success "Cargo.toml updated"

# Update AUR PKGBUILD
log "Updating aur/PKGBUILD..."
sed -i "s/^pkgver=.*/pkgver=$VERSION/" aur/PKGBUILD
success "aur/PKGBUILD updated"

# Update Cargo.lock
log "Updating Cargo.lock..."
cd src-tauri && cargo update -p win11-clipboard-history-lib --precise "$VERSION" 2>/dev/null || cargo check 2>/dev/null || true
cd ..
success "Cargo.lock updated"

# Show what changed
echo ""
log "Version changes:"
echo "  package.json:      $(grep '"version"' package.json | head -1 | sed 's/.*: "\(.*\)".*/\1/')"
echo "  tauri.conf.json:   $(grep '"version"' src-tauri/tauri.conf.json | head -1 | sed 's/.*: "\(.*\)".*/\1/')"
echo "  Cargo.toml:        $(grep '^version' src-tauri/Cargo.toml | head -1 | sed 's/.*= "\(.*\)"/\1/')"
echo "  aur/PKGBUILD:      $(grep '^pkgver=' aur/PKGBUILD | sed 's/pkgver=//')"
echo ""

# Commit version bump
log "Committing version bump..."
git add package.json package-lock.json src-tauri/tauri.conf.json src-tauri/Cargo.toml src-tauri/Cargo.lock aur/PKGBUILD 2>/dev/null || true
git add -A
git commit -m "chore: bump version to $VERSION" || warn "Nothing to commit (version already set?)"

# Create tag
log "Creating tag v$VERSION..."
if git rev-parse "v$VERSION" >/dev/null 2>&1; then
    warn "Tag v$VERSION already exists!"
    read -p "Delete and recreate? [y/N] " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        git tag -d "v$VERSION"
        git push origin --delete "v$VERSION" 2>/dev/null || true
    else
        error "Tag already exists. Aborting."
    fi
fi
git tag -a "v$VERSION" -m "Release v$VERSION"
success "Tag v$VERSION created"

# Push
log "Pushing to origin..."
git push origin "$BRANCH"
git push origin "v$VERSION"
success "Pushed to origin"

echo ""
echo -e "${GREEN}╔════════════════════════════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║  🚀 Release v$VERSION initiated!                              ${NC}"
echo -e "${GREEN}╠════════════════════════════════════════════════════════════════╣${NC}"
echo -e "${GREEN}║  GitHub Actions will now:                                      ${NC}"
echo -e "${GREEN}║  • Build .deb, .rpm, and .AppImage                            ${NC}"
echo -e "${GREEN}║  • Create GitHub Release                                       ${NC}"
echo -e "${GREEN}║  • Upload to Cloudsmith                                        ${NC}"
echo -e "${GREEN}║  • Update AUR package                                          ${NC}"
echo -e "${GREEN}╚════════════════════════════════════════════════════════════════╝${NC}"
echo ""
echo "Track progress: https://github.com/gustavosett/Windows-11-Clipboard-History-For-Linux/actions"
