# fcitx-autotheme

监听 Plasma 主题和配色方案变化，自动重新生成 fcitx5 主题以保持外观一致 — 无需手动操作。

因为目前 flatpak 中的 fcitx5 不会自动匹配 Plasma 主题，所以这个项目因此诞生。

## AI

该项目全程使用AI制作：ChatGPT制作脚本的核心部分，DeepSeek V4 Pro + [OpenCode](https://github.com/anomalyco/opencode) + [oh-my-openagent](https://github.com/code-yeongyu/oh-my-openagent) 负责 Rust 部分和 README。README有经过人工修改。

需要注意的事，我没有 Rust 编程经验（事实上，几乎一点都没学 :D），所以不能保证该项目不会出一些很傻缺的问题。

## 功能

当你切换 Plasma 配色方案或全局主题时，`fcitx-autotheme` 会通过 D-Bus 检测到变化，然后：

1. 运行 `fcitx5-plasma-theme-generator` 生成匹配的 fcitx5 主题
2. 重新加载 fcitx5 的 `classicui` 插件，使新主题立即生效

这是对[原始 bash 脚本](watch-theme-then-update-fcitx-theme)的完整 Rust 复刻，
使用原生异步 I/O、优雅的退出处理和信号防抖机制。

## 构建

```bash
cargo build --release
```

### 依赖

- `fcitx5-plasma-theme-generator`（可通过包管理器或 AUR 安装）
- 运行中的 D-Bus 会话总线
- KDE Plasma 桌面环境（用于监听相应的 D-Bus 信号）

## 用法

```
fcitx-autotheme [OPTIONS]

Options:
  -w, --wait-time <MILLIS>  处理前的防抖等待时间（毫秒）[default: 100]
  -h, --help                打印帮助信息
```

使用自定义的防抖时间（例如等待 500 毫秒再响应）：

```bash
fcitx-autotheme --wait-time 500
```

## 工作原理

`fcitx-autotheme` 监听两个 D-Bus 信号：

| 信号 | 触发条件 |
|---|---|
| `org.freedesktop.portal.Settings.SettingChanged`，命名空间为 `org.kde.kdeglobals.General`，键为 `ColorScheme` | KDE 配色方案变化 |
| `org.kde.kconfig.notify.ConfigChanged`，路径为 `/plasmarc` | Plasma 主题/配置变化 |

信号采用**防抖处理**：当检测到变化时，守护进程会等待一段可配置的静默期（默认 100 毫秒）。
在此期间到达的任何额外信号都会重置计时器。当静默窗口结束且无新信号时，主题仅重新生成一次。
这避免了多个设置同时变化（例如同时切换配色方案和 Plasma 主题）时的重复处理。

收到退出信号（SIGINT / SIGTERM）时，守护进程会优雅退出。

## 许可证

See [LICENSE](./LICENSE)
