# 项目知识库

## 项目概览

- 本仓库是 `vgpu_unlock-rs`，Rust 2018 crate，`crate-type = ["cdylib"]`。
- 产物是共享库，部署模型是通过 `LD_PRELOAD` 注入 NVIDIA vGPU 相关服务，配合上游 `vgpu_unlock` 内核补丁使用。
- 默认 feature 包含 `proxmox`，相关逻辑会按 VMID 查找配置覆盖。
- 这份文档是后续 agent 的维护指南，不是 README 安装教程。

## 目录导览

- `src/lib.rs` 是运行时核心入口，包含 `#[ctor]` 配置加载和 `unsafe extern "C" fn ioctl` hook。
- `src/config.rs` 定义全局配置结构、默认值和 PCI ID 反序列化规则。
- `src/nvidia/` 保存 NVIDIA 低层协议绑定，结构体布局、控制码和常量风险高，另见该目录自己的指南。

## 常用命令

- 构建共享库：`cargo build --release`。
- 可尝试的测试命令：`cargo test`。只把它当作建议验证命令，不要在未运行时写成事实。
- 本仓库未发现 Makefile、Justfile、CI workflow 或专用测试目录。不要虚构相关命令或 CI 流程。

## 配置与运行模型

- 主配置路径是 `/etc/vgpu_unlock/config.toml`，profile 覆盖路径是 `/etc/vgpu_unlock/profile_override.toml`。
- `src/config.rs` 的默认值是 `unlock = true`，`unlock_migration = false`。
- `spoofed_devid` 和 `spoofed_subsysid` 接受十进制整数，也接受 `0x...` 或 `0X...` 字符串。
- `/etc/vgpu_unlock/profile_override.toml` 中的 `[custom]` 覆盖优先于主配置中的同名 PCI ID 覆盖。

## 维护注意事项

- 修改 `ioctl` hook、FFI 签名、NVIDIA 控制结构或常量前，先查 `src/lib.rs` 和 `src/nvidia/` 内引用。
- Rust 源文件使用 `// SPDX-License-Identifier: MIT` header 风格，新 Rust 文件应保持一致。
- 保持代码标识符、TOML key、路径和命令原样书写，不要翻译。
- 文档中描述验证结果时要谨慎，只记录实际运行过的命令和观察到的输出。

## 不要做

- 不要复制 README 的安装步骤全文到维护指南。
- 不要声称 `cargo build`、`cargo test`、rust-analyzer 或 CI 在当前环境完成验证，除非本轮确实运行并记录了结果。
- 不要新增依赖、改动 Cargo 配置或 Rust 源码来完成文档任务。
- 不要创建额外的根级或子目录 `AGENTS.md`，除非计划明确要求。
