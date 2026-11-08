#!/usr/bin/env bash
# Build libFLAC 1.5.x from source into /usr/local for the CI Linux jobs.
#
# ubuntu-22.04's apt libflac-dev is 1.3.4, but the copy-in ld-compress-ng
# uses the 1.5 chained-Ogg-FLAC decoder APIs (FLAC__STREAM_DECODER_END_OF_
# LINK, FLAC__stream_decoder_set_decode_chained_stream). MSYS2 and Homebrew
# already ship 1.5.x, so only the Linux runners need this.
#
# Usage (after apt-get install -y libogg-dev):
#   scripts/ci_build_flac.sh [version]     # default 1.5.0
#
# Exports PKG_CONFIG_PATH=/usr/local/lib/pkgconfig via GITHUB_ENV so the
# ld-compress-ng subproject's pkg_check_modules(FLAC) resolves THIS build,
# and runs ldconfig so spawned binaries find /usr/local/lib at runtime.
set -eux

FLAC_VERSION="${1:-1.5.0}"

# Prefer Xiph's canonical source archive endpoint: the GitHub release
# asset URL has returned HTTP 404 on GitHub Actions Linux runners.
curl -fL --retry 3 --retry-delay 2 --retry-all-errors \
  "https://downloads.xiph.org/releases/flac/flac-${FLAC_VERSION}.tar.xz" \
  -o /tmp/flac.tar.xz

# Pin the published 1.5.0 SHA-256, so CI never compiles a wrong or partial
# archive. Other versions still require an explicitly maintained checksum.
case "${FLAC_VERSION}" in
  1.5.0)
    printf '%s  %s\n' \
      'f2c1c76592a82ffff8413ba3c4a1299b6c7ab06c734dee03fd88630485c2b920' \
      /tmp/flac.tar.xz | sha256sum --check --status
    ;;
  *)
    echo "Unsupported FLAC version without a pinned checksum: ${FLAC_VERSION}" >&2
    exit 1
    ;;
esac

rm -rf "/tmp/flac-${FLAC_VERSION}" /tmp/flac-build
tar -xf /tmp/flac.tar.xz -C /tmp

# Shared libs into /usr/local; Ogg support ON (the 1.5 chained-stream API
# is the Ogg-FLAC decoder path) — libogg-dev must be installed beforehand.
cmake -S "/tmp/flac-${FLAC_VERSION}" -B /tmp/flac-build -G Ninja \
  -DCMAKE_BUILD_TYPE=Release \
  -DBUILD_SHARED_LIBS=ON \
  -DCMAKE_INSTALL_PREFIX=/usr/local
sudo cmake --build /tmp/flac-build --target install
sudo ldconfig
/usr/local/bin/flac --version

echo "PKG_CONFIG_PATH=/usr/local/lib/pkgconfig:${PKG_CONFIG_PATH:-}" >> "$GITHUB_ENV"
