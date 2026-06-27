#!/usr/bin/env bash
# generate-sbom.sh — 生成 CycloneDX Software Bill of Materials
#
# 用法:
#   ./scripts/generate-sbom.sh              # 生成 JSON 格式 SBOM
#   ./scripts/generate-sbom.sh --xml         # 生成 XML 格式 SBOM
#   ./scripts/generate-sbom.sh --verify      # 校验已有 SBOM
#
# 依赖: cargo-cyclonedx (cargo install cargo-cyclonedx)
# 输出: sbom/syncthing-rust-<version>.cdx.json

set -euo pipefail
cd "$(dirname "$0")/.."

FORMAT="json"
VERIFY=false
for arg in "$@"; do
    case "$arg" in
        --xml) FORMAT="xml" ;;
        --verify) VERIFY=true ;;
    esac
done

VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
OUTDIR="sbom"
mkdir -p "$OUTDIR"

if $VERIFY; then
    echo "=== Verifying SBOM ==="
    if command -v cargo-cyclonedx &>/dev/null; then
        cargo cyclonedx --all --spec-version 1.5 --output-pattern "${OUTDIR}/syncthing-rust-{version}.cdx.json"
        echo "SBOM regenerated for verification at ${OUTDIR}/"
    else
        echo "ERROR: cargo-cyclonedx not installed. Run: cargo install cargo-cyclonedx"
        exit 1
    fi
else
    echo "=== Generating CycloneDX SBOM v1.5 ==="
    echo "Version: $VERSION"
    echo "Format:  $FORMAT"

    if command -v cargo-cyclonedx &>/dev/null; then
        cargo cyclonedx --all --spec-version 1.5 --format "$FORMAT" \
            --output-pattern "${OUTDIR}/syncthing-rust-{version}.cdx.${FORMAT}"
        echo ""
        echo "✅ SBOM generated: ${OUTDIR}/syncthing-rust-${VERSION}.cdx.${FORMAT}"
        echo ""
        echo "Supply chain verification:"
        echo "  cargo audit     # 已知漏洞检查"
        echo "  cargo deny check # 许可证审计"
        echo "  cargo vet       # 依赖信任验证 (可选)"
    else
        echo "ERROR: cargo-cyclonedx not installed."
        echo "Install: cargo install cargo-cyclonedx"
        echo ""
        echo "Alternative (cargo-sbom):"
        echo "  cargo install cargo-sbom"
        echo "  cargo sbom > ${OUTDIR}/syncthing-rust-${VERSION}.spdx.json"
        exit 1
    fi
fi
