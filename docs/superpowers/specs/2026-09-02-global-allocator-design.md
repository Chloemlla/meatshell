# 全局内存分配器设计

## 背景

为改善 meatshell 在不同平台上的堆内存分配性能，需要按目标平台选择专用分配器，同时保留不支持平台的可移植性。

## 方案

- Windows 目标使用 `mimalloc`。
- Linux、macOS、Android、iOS 及 FreeBSD、NetBSD、OpenBSD、DragonFly 目标使用 `jemallocator`。
- 其他目标使用标准库 `System` 分配器兜底。
- 依赖在 `Cargo.toml` 中按 `cfg` 条件声明，避免非目标平台编译无关依赖。
- 分配器选择集中在独立模块中，主程序只通过模块加载，确保全工程只有一个 `#[global_allocator]` 声明。

## Example 与测试

新增 `examples/allocator_demo.rs`，复用同一套平台选择逻辑，执行 1024 个 `i32` 的堆分配并输出分配器类别与大小。测试覆盖：

1. 当前目标平台的分配器类别符合映射。
2. `Vec` 分配、写入和释放正常完成。

先运行测试确认缺少实现时失败，再实现最小代码使其通过，随后运行项目测试和 Linux 构建验证回归。

## 兼容性

jemalloc 依赖仅对明确列出的 Unix-like 目标启用；Android 和 iOS 使用与其他 Unix-like 目标相同的配置。WASM、嵌入式等目标继续使用 `System`，不新增依赖。
