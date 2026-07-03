#!/usr/bin/env bash
# LightOS installer — fetch the latest release and install a `lightos`
# launcher for the current user. QEMU-targeted; no root required.
#
#     curl -fsSL https://raw.githubusercontent.com/Lightiam/LightOS_Orion_RISC-V/main/scripts/install.sh | bash
#
# Installs into ~/.lightos and drops a `lightos` command in
# ~/.local/bin. Set LIGHTOS_VERSION to pin a release (default: latest).
set -euo pipefail

REPO="Lightiam/LightOS_Orion_RISC-V"
PREFIX="${LIGHTOS_PREFIX:-$HOME/.lightos}"
BINDIR="${LIGHTOS_BINDIR:-$HOME/.local/bin}"
VERSION="${LIGHTOS_VERSION:-latest}"

need() { command -v "$1" >/dev/null 2>&1 || { echo "install: '$1' is required" >&2; exit 1; }; }
need curl
need tar

# Resolve the download URL for the release tarball.
api="https://api.github.com/repos/$REPO/releases"
if [ "$VERSION" = "latest" ]; then
    api="$api/latest"
else
    api="$api/tags/$VERSION"
fi

echo "install: resolving $VERSION release of $REPO ..."
asset_url=$(curl -fsSL "$api" \
    | grep -oE '"browser_download_url": *"[^"]*lightos-[^"]*\.tar\.gz"' \
    | head -n1 | sed -E 's/.*"(https[^"]*)".*/\1/')

if [ -z "${asset_url:-}" ]; then
    echo "install: no release tarball found (has a release been published yet?)." >&2
    echo "install: you can still build from source: git clone https://github.com/$REPO && make release" >&2
    exit 1
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
echo "install: downloading $(basename "$asset_url") ..."
curl -fsSL "$asset_url" -o "$tmp/lightos.tar.gz"
tar xzf "$tmp/lightos.tar.gz" -C "$tmp"

bundle="$(find "$tmp" -maxdepth 1 -type d -name 'lightos-*' | head -n1)"
[ -n "$bundle" ] || { echo "install: unexpected tarball layout" >&2; exit 1; }
ver="$(cat "$bundle/VERSION" 2>/dev/null || echo unknown)"

mkdir -p "$PREFIX" "$BINDIR"
dest="$PREFIX/lightos-$ver"
rm -rf "$dest"
mv "$bundle" "$dest"
ln -sfn "$dest" "$PREFIX/current"

# `lightos` launcher forwards to the current bundle's run.sh.
cat > "$BINDIR/lightos" <<EOF
#!/usr/bin/env bash
exec "$PREFIX/current/run.sh" "\$@"
EOF
chmod +x "$BINDIR/lightos"

echo
echo "LightOS $ver installed."
echo "  bundle : $dest"
echo "  command: $BINDIR/lightos"
case ":$PATH:" in
    *":$BINDIR:"*) echo "Run it now:  lightos" ;;
    *) echo "Add to PATH first:  export PATH=\"$BINDIR:\$PATH\"   then run:  lightos" ;;
esac
