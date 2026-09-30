# Zero Dock 窗口停靠

[English](README.md) · [安装与升级](docs/INSTALL.md) · [贡献规范](CONTRIBUTING.md)

本项目所有检查在本机执行，不使用 GitHub CI 或 Actions。

C / GTK3 编写的 XFCE 原生面板插件，使用 libxfce4windowing 管理窗口，libpulse 控制 PipeWire-Pulse 应用音频，XComposite 获取窗口缩略图。

## 使用

右键 XFCE 面板 → 面板 → 添加新项目 → **Zero Dock 窗口停靠**。
可添加多个实例，各自保存固定列表和设置。运行于 XFCE `wrapper-2.0` 独立进程；一个插件进程退出不会结束面板或其他实例。需要 X11；实时画面需要运行中的 X11 合成器。

- 每个窗口一个图标按钮，同应用窗口不合并；活动窗口紫色下划线，最小化半透明及圆点，请求注意时黄色描边。
- 左键激活/最小化；中键启动该应用；滚轮按按钮顺序切换窗口。
- 悬停 350 ms 显示真实弹层，每 650 ms 更新；点击画面恢复/激活窗口，中键打开应用新窗口。看预览时面板不会自动隐藏。
- 最小化前先截取画面，活动窗口每 5 秒缓存一帧；仅在内存中保存，关闭窗口立即释放。无缓存时显示占位图标。
- 窗口右键包含恢复、最大化、全屏、置顶、工作区、静音、新窗口、固定、关闭。不支持的操作置灰。
- 右键窗口“固定到启动器”，或从文件管理器拖入 `.desktop` 文件；固定启动器与窗口按钮分别显示。
- 固定图标右键包含桌面动作、位置移动和取消固定；所有按钮都可以拖动排序，蓝线表示插入位置，固定图标相对顺序持久保存。
- 喇叭徽章在右上角：点击静音/取消静音应用，滚轮每格 5%，上限 200%；预览里的喇叭也支持滚轮，连续小幅滚动会累积；音量气泡 1.2 秒后消失。
- 同应用多个窗口的序号位于左上角。右键插件 → 属性可以关闭序号、关闭预览，或仅显示当前工作区。
- 默认预留 10 个图标位置，减少窗口开关时面板位置漂移；属性中可设为 0（自动宽度）或 1–32。
- 插件空白处右键：显示桌面、最小化所有窗口，及 XFCE 标准属性/关于/移动/移除菜单。菜单状态同步不会重复触发显示桌面操作。

配置文件：`~/.config/xfce4/panel/zero-dock-<实例ID>.rc`。GTK 原生图标和主题来自系统设置，插件不更改全局 GTK 配置。

## 边界

音频优先匹配进程祖先链；无法对应窗口时再按可执行名、窗口类名匹配。Chrome 子进程原理有覆盖，gamescope 等包装器取决于实际进程关系和应用元数据；没有验证所有浏览器版本和游戏。共享音频进程的多个窗口按应用控制，不保证能单独静音某个浏览器标签页。

喇叭以“音频流未暂停/已静音”为依据，不做音频波形分析。保持音频流打开但输出静音数据的程序也可能显示徽章。没有匹配到 `.desktop` 文件的窗口仍可管理，但“启动新窗口”“固定”菜单项会置灰；可以直接拖入该应用的桌面文件。

已有最小化窗口若在插件启动前未缓存，会先显示占位图标；恢复后即可生成画面。预览不包含 DRM 保护内容的显示保证。

## 构建与验证

```bash
meson setup build --prefix=/usr --libdir=lib -Dtests=true
meson compile -C build
meson test -C build --print-errorlogs
python tests/run-isolated.py
```

测试脚本使用 Xvfb + 独立 xfwm4 + 私有 D-Bus，不操作当前桌面窗口。音频集成测试创建自己的零音量音频流，只修改该测试流，结束后清理。

需要：xfce4-panel、libxfce4ui、libxfce4windowing、gtk3、glib2、libpulse、libxcomposite、libx11、libxi、meson、ninja。测试另需 libxtst、xorg-server-xvfb、xfwm4、Python、pacat。Python 只用于隔离测试驱动，已安装插件没有 Python 运行依赖。

Arch/CachyOS 本地打包见 `packaging/PKGBUILD`。使用 pacman 安装/卸载，不覆盖发行版的 tasklist 插件。

## 回退

右键 Zero Dock → 移除，然后在“添加新项目”中添加 XFCE 原生“窗口按钮”，关闭分组、关闭标签。这个过程无需重启面板、退出 XFCE 或重启登录器。需要卸载时：`sudo pacman -R xfce4-zero-dock-plugin`。

如插件退出，XFCE 可单独重启该插件；不要运行 `xfce4-panel --restart` 或终止 XFCE 会话。

开源发布、问题报告、测试范围和架构分别见 [发布流程](docs/RELEASING.md)、[Issues](https://github.com/lunemoe/zero-dock/issues)、[测试指南](docs/TESTING.md)、[架构说明](docs/ARCHITECTURE.md)。
