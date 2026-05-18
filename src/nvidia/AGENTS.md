# NVIDIA 协议绑定维护指南

## 目录职责

- 本目录保存 NVIDIA 低层协议绑定，包括结构体布局、ioctl 请求、错误码、控制码和 RM control 参数。
- 这些文件只描述仓库中已使用或已绑定的协议片段，不代表完整 NVIDIA 私有协议知识。
- 通用构建、配置和项目范围规则见根 `AGENTS.md`，这里不重复。

## 文件导览

- `mod.rs` 导出所有 NVIDIA 控制模块，包括 `ctrl0000vgpu`、`ctrl0080gpu`、`ctrl2080bus`、`ctrl2080gpu`、`ctrl9096`、`ctrla081`、`ctrla082`、`error`、`ioctl`、`nvos` 和 `nvtypes`。
- `ioctl.rs` 定义 `NV_IOCTL_MAGIC`，用于派生 NVIDIA ioctl request 值。
- `nvos.rs` 定义 `NV_ESC_RM_CONTROL` 和 `Nvos54Parameters`，对应 RM control ioctl payload。
- `error.rs` 保存 NVIDIA status/error 常量，包含 `NV_OK`、`NV_ERR_BUSY_RETRY` 等 `src/lib.rs` 会直接使用的状态码。
- `ctrl0000vgpu.rs`、`ctrla081.rs`、`ctrla082.rs`、`ctrl0080gpu.rs`、`ctrl2080bus.rs`、`ctrl2080gpu.rs`、`ctrl9096.rs` 是控制命令和参数绑定。
- `nvtypes.rs` 提供 NVIDIA 基础类型别名，修改前要确认所有绑定结构的类型需求。

## 维护规则

- 保留模块名、公开结构体、公开常量和控制码原名，除非已检查 `src/lib.rs` 和本目录内所有引用。
- `#[repr(C)]` 与 `#[repr(C, align(8))]` 结构体编码 ABI 敏感布局，字段顺序、字段类型和对齐包装都要按驱动期望处理。
- 维护 `AlignedU64`、固定长度数组和 `Uuid` 等字段时，先确认它们对结构体大小与字段偏移的影响。
- 一些绑定模块带有内联 size tests，`mem::size_of` 断言用于保护布局假设；改布局时必须同步审查这些断言。
- 注释中的来源信息只说明当前绑定的参考背景，不要据此添加未在代码中验证过的同步流程。

## 高风险修改

- 修改字段顺序、字段类型、alignment wrapper、数组长度、常量值或控制码都可能破坏 ABI 或驱动协议兼容。
- 改动 `NV_IOCTL_MAGIC`、`NV_ESC_RM_CONTROL`、`Nvos54Parameters`、`NV_OK`、`NV_ERR_BUSY_RETRY` 前，先追踪 `src/lib.rs` 的 ioctl hook 使用路径。
- 改动 `NVA081_CTRL_CMD_*`、`NVA082_CTRL_CMD_*`、`NV0000_CTRL_CMD_*` 等控制码前，先确认调用侧如何匹配 `cmd`。
- 调整 Debug 输出通常比改协议字段安全，但也要避免隐藏调试所需的关键常量或 payload 内容。

## 不要做

- 不要随意重命名公开结构体、常量或模块。
- 不要把根 `AGENTS.md` 的项目概览、构建命令或配置路径段落复制到这里。
- 不要把 README 安装步骤、systemd 配置或 profile 示例写入本目录指南。
- 不要为了消除告警而删除 status/error 常量，许多常量是协议枚举的一部分。
- 不要声称已掌握完整 NVIDIA 私有协议；只按仓库现有绑定和注释维护。
