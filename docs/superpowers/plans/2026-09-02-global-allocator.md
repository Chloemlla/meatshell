# 全局内存分配器实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 `superpowers:executing-plans` 在独立工作树中逐任务执行此计划。

**目标：** 为 meatshell 按目标平台启用 mimalloc 或 jemallocator，并用 example 验证选择逻辑和实际堆分配。

**架构：** 平台选择集中在 `src/allocator.rs`，主程序通过 `mod allocator` 注册唯一全局分配器。`examples/allocator_demo.rs` 独立复用同样的条件映射，用单元测试验证平台名称并执行 `Vec` 分配。

**技术栈：** Rust `#[cfg]`、`#[global_allocator]`、`mimalloc 0.1`、`jemallocator 0.5`、Cargo 条件依赖。

---

### 任务 1：配置平台依赖

**文件：**
- 修改：`Cargo.toml`
- 修改：`Cargo.lock`

- [ ] **步骤 1：** 在 Windows target dependencies 添加 `mimalloc = "0.1"`，在 Linux、macOS、Android、iOS 和 BSD target dependencies 添加 `jemallocator = "0.5"`。
- [ ] **步骤 2：** 运行 `cargo check --example allocator_demo`，确认依赖解析可用；若 example 尚未实现则允许因源码缺失失败。

### 任务 2：实现并验证 allocator example

**文件：**
- 修改：`examples/allocator_demo.rs`

- [ ] **步骤 1：** 保留平台名称测试，补充统一 `Allocator` 类型、`GLOBAL` 全局分配器、`allocator_name()` 和 1024 元素 `Vec` 分配。
- [ ] **步骤 2：** 运行 `cargo test --example allocator_demo`，预期测试通过。
- [ ] **步骤 3：** 运行 `cargo run --example allocator_demo`，预期输出 jemalloc（Linux）及 4096 bytes。

### 任务 3：接入 meatshell 主程序

**文件：**
- 创建：`src/allocator.rs`
- 修改：`src/main.rs`

- [ ] **步骤 1：** 将平台 cfg 和 `#[global_allocator]` 集中到 `src/allocator.rs`，主程序以 `mod allocator;` 引入。
- [ ] **步骤 2：** 运行 `cargo check`，确认主程序只有一个全局分配器且各模块编译通过。

### 任务 4：回归验证和提交

**文件：** 无新增文件。

- [ ] **步骤 1：** 运行 `cargo test --example allocator_demo` 和 `cargo test`。
- [ ] **步骤 2：** 运行 `cargo build --release`，确认 Linux 构建成功。
- [ ] **步骤 3：** 检查 `git diff` 和 `git status`，确认无临时文件或无关改动。
- [ ] **步骤 4：** 提交：`feat(构建): 按平台启用全局内存分配器`。
