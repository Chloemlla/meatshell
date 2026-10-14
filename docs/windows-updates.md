# Windows 应用内更新

Windows x64 MSI 安装版可在设置的更新页面检查版本、下载更新，然后选择“安装并重启”。启动时的自动检查开关只控制自动检查；手动检查始终可用。新版本提示条与设置页使用同一更新状态，多窗口不会重复发起下载或安装。

应用通过 GitHub 的最新正式发布接口获取版本，只选择对应版本的官方 Windows x64 MSI。下载按发布元数据校验大小和 SHA-256，安装辅助进程在接管时再次验证文件。网络失败、包不完整、校验不匹配或发布中尚未提供 MSI 时，程序继续运行并显示可重试的错误；发布页面仍可手动打开。

安装前会要求用户确认关闭所有窗口和连接，并提醒保存编辑中的文件。辅助进程就绪后，应用使用已有退出流程保存窗口布局、停止连接并退出；辅助进程等待退出完成后，调用 Windows Installer 安装 MSI。覆盖升级、原安装位置和安装回滚由现有 MSI 定义负责。安装可能需要管理员授权，不会自动重启 Windows。

安装结束或取消后，辅助进程删除本次下载包及辅助临时文件，并尝试从原安装位置重新启动应用，保留当前数据目录。安装失败会显示 Windows Installer 返回的错误。若安装返回需要重启 Windows，程序会提示用户。

临时文件位于应用缓存目录的 `updates/<UUID>/` 下。下载和安装各自持有同一目录的共享读取句柄；清理需要独占访问，因此不会删除仍在进行中的更新。正常退出会清理未安装的下载；下次启动会重试清理异常退出或文件暂时被占用留下的文件。清理只删除已知临时文件，不递归删除目录内容，也不触碰 Windows Installer 系统缓存或用户配置。

便携版、源码运行和其他系统保留通过发布页面更新的方式。应用内安装要求当前程序路径与 MSI 写入的安装位置匹配。

## Windows 运行依赖

更新模块使用 Windows 10 / Windows 11 x64 都提供的系统组件。辅助脚本调用系统自带的 Windows PowerShell 5.1 和 .NET Framework，通过 `Start-Process -Verb RunAs -Wait` 启动 `msiexec.exe /i /passive /norestart`。脚本保留 UTF-8 BOM，以正确读取中文文本；无需 PowerShell 7、额外安装的 .NET 或 winget。[PowerShell 5.1](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_windows_powershell_5.1)、[Start-Process 5.1](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.management/start-process?view=powershell-5.1) 和 [msiexec](https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/msiexec) 的要求见微软文档。

安装位置检测使用 `RegGetValueW`，临时文件互斥使用 Windows 文件共享模式；这些接口在 Windows 10 上可用。下载沿用项目已有的 `ureq` 和 SHA-256 依赖。更新代码不依赖 Windows 11 专属接口，也不按系统名称或版本号选择安装路径。

当前真实 MSI 测试环境为 Windows 11 x64、Windows PowerShell 5.1。Windows 10 的接口和脚本兼容性已经核对，Windows 10 真机或虚拟机上的安装、回滚和清理仍待验证；此核对范围仅为新增更新模块。

## 代码位置

- `src/update.rs`：发布信息、安装包选择和流式下载校验。
- `src/update/windows.rs`：识别 MSI 安装、暂存文件、辅助进程握手和清理。
- `src/update/install.ps1`：等待退出、启动 MSI、清理和重新打开应用。文件使用 UTF-8 BOM，以兼容 Windows PowerShell 5.1。
- `src/app/update.rs`：共享更新状态以及 Slint 回调。

## 定向验证

```powershell
cargo check --locked --bin meatshell
cargo test --locked --bin meatshell update::
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tests/update/installer.ps1 -ScratchDirectory <临时测试目录>
```

Rust 用例覆盖正式版本比较、安装包来源、校验值、下载中断、磁盘写入失败及清理锁。PowerShell 用例用模拟安装器覆盖成功、需要重启、取消、安装失败、管理员授权取消、校验失败和重启参数转义，不会安装软件。

真实 MSI 测试需要 Windows、WiX 3、MSVC Rust 工具链以及系统自带的 .NET Framework 编译器：

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tests/update/msi.ps1 -WixBin <WiX目录> -ScratchDirectory <临时测试目录>
```

脚本从 `wix/main.wxs` 生成独立产品标识的测试 MSI，载荷为可记录启动版本的测试程序，使用实际更新辅助脚本和 Windows Installer 执行安装。测试覆盖成功升级、文件替换后取消（1602）、文件替换后失败（1603）及失败后重试；取消和失败由 MSI 内的延迟操作触发，安装回滚由 Windows Installer 实际执行。

核对内容包括磁盘和重新启动的版本、产品注册状态、自定义安装位置、含空格和中文的数据目录、配置文件以及每次更新暂存目录的删除。测试需要管理员授权，结束时卸载测试产品；构建产物、安装日志和 `result.json` 保留在指定测试目录。这组测试覆盖安装链路，真实 MeatShell 界面交互和人工拒绝 UAC 的操作仍需交互验收。
