#!/usr/bin/env bash
# 文件变更生成器：在指定目录中持续创建/修改/删除文件，模拟真实使用

set -euo pipefail

TARGET_DIR="${1:-/tmp/syncthing-churn}"
mkdir -p "$TARGET_DIR"

# 文件大小分布（字节）
declare -a SIZES=(1024 10240 102400 1048576 10485760)

# 操作间隔（秒）
INTERVAL=30

echo "[churn] Starting file churn in $TARGET_DIR (interval: ${INTERVAL}s)"

counter=0
while true; do
    counter=$((counter + 1))
    op=$((RANDOM % 3))
    size=${SIZES[$((RANDOM % ${#SIZES[@]}))]}
    filename="file_$(date +%s)_${counter}.bin"
    filepath="$TARGET_DIR/$filename"

    case $op in
        0)
            # 创建文件
            dd if=/dev/urandom of="$filepath" bs=1 count=$size 2>/dev/null
            echo "[churn] CREATE $filename ($size bytes)"
            ;;
        1)
            # 修改现有文件
            files=("$TARGET_DIR"/*)
            if [[ ${#files[@]} -gt 0 && -f "${files[0]}" ]]; then
                target="${files[$((RANDOM % ${#files[@]}))]}"
                if [[ -f "$target" ]]; then
                    dd if=/dev/urandom of="$target" bs=1 count=$size conv=notrunc 2>/dev/null
                    echo "[churn] MODIFY $(basename "$target") ($size bytes)"
                fi
            fi
            ;;
        2)
            # 删除文件
            files=("$TARGET_DIR"/*)
            if [[ ${#files[@]} -gt 0 && -f "${files[0]}" ]]; then
                target="${files[$((RANDOM % ${#files[@]}))]}"
                if [[ -f "$target" ]]; then
                    rm -f "$target"
                    echo "[churn] DELETE $(basename "$target")"
                fi
            fi
            ;;
    esac

    sleep $INTERVAL
done
