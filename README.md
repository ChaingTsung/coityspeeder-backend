# ⚡ CoitySpeeder Backend Core

CoitySpeeder 的核心测速引擎。使用 Rust 编写，集成了极速并发测试、内核智能调度（Xray / Mihomo）、流媒体/AI 解锁精准检测等功能。

## ✨ 核心特性

- **🦀 Rust 极致性能**：基于 Tokio 和 Axum 构建的高并发异步运行时，内存占用极低。
- **🧠 智能内核调度**：自动识别 `vless://` 直链或 `yaml` 订阅，动态拉起对应的 Xray / Mihomo 内核进程，测速完毕后精准回收。
- **🔓 全能解锁检测**：并发检测 IP 风险度（欺诈评分）、Netflix、ChatGPT、Claude、Gemini 的解锁状态。
- **🔄 CLI & API 双模式**：既可以作为守护进程提供 HTTP API 供前端调用，也可作为 CLI 命令行工具独立运行。

## ⚠️ 目录结构要求 (极其重要)

后端在运行时，**必须**能在其上一级目录找到 Xray 和 Mihomo 的可执行文件！标准的生产环境目录结构应如下：

\`\`\`text
/opt/CoitySpeeder/
├── backend/
│   └── coityspeeder     <-- 本项目的 Rust 二进制文件
├── xray                 <-- Xray-core 可执行文件 (需赋予 +x 权限)
└── mihomo               <-- Mihomo 可执行文件 (需赋予 +x 权限)
\`\`\`

## 🛠️ 编译与运行

请确保已安装 Rust 稳定版工具链。

\`\`\`bash
# 1. 编译 Release 版本
cargo build --release

# 2. 启动 API 服务器模式 (默认挂载 0.0.0.0:3000)
./target/release/coityspeeder server --bind 0.0.0.0:3000

# 3. CLI 命令行单次测速模式
./target/release/coityspeeder test --target "vless://xxxxx"
\`\`\`

## 📦 生产部署 (Systemd 守护进程)

推荐使用 Systemd 守护运行后端服务。修改 `/etc/systemd/system/coityspeeder.service`：

\`\`\`ini
[Unit]
Description=CoitySpeeder Backend
After=network.target

[Service]
Type=simple
# 注意：工作目录必须设为 backend 文件夹内
WorkingDirectory=/opt/CoitySpeeder/backend
ExecStart=/opt/CoitySpeeder/backend/coityspeeder server --bind 127.0.0.1:3000
Restart=on-failure
Environment=RUST_LOG=info

[Install]
WantedBy=multi-user.target
\`\`\`

*注意：如果前端与后端不在同一域名下，请务必在 Nginx 的反向代理块中配置 CORS 允许跨域。*