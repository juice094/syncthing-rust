#!/usr/bin/env bash
# generate-sbom.sh — 生成 CycloneDX Software Bill of Materials
#
# 用法:
#   ./scripts/generate-sbom.sh              # 生成 JSON 格式 SBOM
#   ./scripts/generate-sbom.sh xml           # 生成 XML 格式 SBOM
#
# 依赖: cargo-cyclonedx (cargo install cargo-cyclonedx)
# 输出: sbom/*.cdx.{json,xml} (每个 crate/target 一个文件)

set -euo pipefail
cd "$(dirname "$0")/.."

FORMAT="${1:-json}"
OUTDIR="sbom"

if ! command -v cargo-cyclonedx &>/dev/null; then
    echo "ERROR: cargo-cyclonedx not installed."
    echo "Install: cargo install cargo-cyclonedx"
    exit 1
fi

VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')
mkdir -p "$OUTDIR"
# 清除旧 SBOM
rm -f "$OUTDIR"/*.cdx."$FORMAT"

echo "=== Generating CycloneDX SBOM ==="
echo "Version: $VERSION"
echo "Format:  $FORMAT"
echo ""

cargo cyclonedx --all -f "$FORMAT"

# cargo-cyclonedx 在每个 crate/cmd 目录生成 .cdx.{format} 文件
# 收集到 sbom/ 目录
find . -name "*.cdx.${FORMAT}" -type f | while read f; do
    mv "$f" "$OUTDIR/"
done

count=$(ls -1 "$OUTDIR"/*.cdx."$FORMAT" 2>/dev/null | wc -l)
echo ""
echo "=== SBOM generated: $count files in $OUTDIR/ ==="
ls -la "$OUTDIR"/

echo ""
echo "Supply chain verification:"
echo "  cargo audit       # 已知漏洞检查"
echo "  cargo deny check  # 许可证审计"
