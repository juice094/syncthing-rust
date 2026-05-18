# 设备ID生成算法修复规范

## 问题分析

### 问题1: 非法字符
Rust实现生成的ID包含 `0, 1, 8, 9`，但Syncthing Base32字符集只包含:
```
ABCDEFGHIJKLMNOPQRSTUVWXYZ234567
```

### 问题2: 校验位算法错误
正确的算法:
- 52个数据字符
- 分成4组×13字符
- 每组独立计算Luhn-32校验位并追加

## 正确算法参考

### Base32编码
使用标准RFC4648 Base32:
- 字符集: `ABCDEFGHIJKLMNOPQRSTUVWXYZ234567`
- 无填充

### Luhn-32算法
```go
// Go原版参考
func luhnify(s string) string {
    for i := 0; i < 4; i++ {
        p := s[i*13 : (i+1)*13]
        l := luhn32(p)
        s = s[:i*13+13] + string(alphabet[l]) + s[(i+1)*13:]
    }
    return s
}

func luhn32(s string) uint32 {
    factor := uint32(1)
    sum := uint32(0)
    for i := len(s) - 1; i >= 0; i-- {
        code := alphabetReverse[s[i]]
        addend := factor * code
        factor = 3 - factor // 在1和2之间切换
        sum += addend
    }
    remainder := sum % 32
    check := (32 - remainder) % 32
    return check
}
```

## 修复位置

`syncthing-core/src/types.rs` - `DeviceId` 生成逻辑

## 验收标准

1. 生成的ID只包含合法字符A-Z2-7
2. Go原版能正确验证校验位
3. 云端Syncthing能成功添加设备
