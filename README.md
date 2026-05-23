# ProjectYing

ProjectYing 是 AITermux 的终端 AI 工作台，面向 Termux/Android 环境开发。

它不是通用桌面程序，也不建议普通用户直接尝试。发布仓库只提供源码与空白初始化模板；API Key、记忆、上下文、日志和构建产物不会随仓库发布。

## 要求

- Android + Termux
- 已安装 AITermux
- 基础 Linux/Termux 使用能力
- 能理解 API Key、模型代理、文件权限和终端日志
- 部分手机自动化、ADB、服务器管理能力可能需要额外授权、无线调试或 root 环境

## 启动

通常由 AITermux 启动器拉起：

```bash
cd ~/AItermux/projectying
./run.sh
```

首次启动会在本地生成运行配置、上下文、记忆和日志。不要把这些运行态文件提交到仓库。

## 本地构建

```bash
pkg install rust clang make pkg-config
cargo build --release
```

## 仓库边界

- 只支持 Termux/AITermux 链路。
- 不承诺在 Linux 桌面、Windows、macOS 或普通 Android shell 中运行。
- 不包含任何 API Key、聊天历史、memory、Aidebug 运行日志或 target 构建产物。
- 使用前先阅读源码和配置，确认你知道它会调用哪些工具。
