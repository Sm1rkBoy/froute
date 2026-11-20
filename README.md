# froute

一个只读的 IPv4 路由表查看工具，将 macOS、Linux、Windows 的系统路由表转换为统一的列格式。掩码长度单独显示为 CIDR 前缀数字，例如 `255.255.255.0` 显示为 `24`，不与网络号拼接：

```text
掩码长度    网络号    下一跳    出接口    度量
```

## 运行

需要安装 Rust 工具链。进入项目目录后执行：

```sh
cargo run
```

程序通过系统自带命令读取路由表，不需要管理员权限，也不会修改网络配置。Windows 使用 `route print -4`，macOS 使用 `netstat -rn`，Linux 优先使用 `ip -4 route show`，并尝试兼容 `netstat` 输出。

## 当前范围与注意事项

- 当前只处理 IPv4。
- 输出为 UTF-8 对齐文本；暂未提供 CSV/JSON 参数。
- macOS 的 `netstat` 不提供可跨版本可靠读取的 metric 列，因此该列显示 `-`。
- 需要对应系统命令可用；精简 Linux 环境若没有 `ip`，需安装提供 `netstat` 的工具包。
- 目前只在 Linux 环境验证实际系统读取；macOS 和 Windows 解析器有单元测试，仍建议在目标系统实测。

## GitHub Actions 构建

推送代码或创建 Pull Request 后，GitHub Actions 会自动构建 Linux、Windows，以及 macOS Apple Silicon（aarch64）和 Intel（x86_64）版本。构建完成后，在仓库的 **Actions → 对应工作流运行 → Artifacts** 下载对应平台的二进制文件。Artifact 默认保留 14 天。

## 开发与测试

```sh
cargo fmt -- --check
cargo test
```
