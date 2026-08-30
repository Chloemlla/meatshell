# MeatShell 架构缺陷审计报告（2026-08-30）

> 依据 `code-audit-methodology.md` 六类雷达（并发/资源/生命周期/编译结构/安全/持久化），8 个并行只读 subagent 分组审查全部 `src/*.rs`（约 3.2 万行）。本文档是**修复落地对照**：每条缺陷含 文件+行号+类别+严重度+问题+场景+修复建议，修复完成后逐条更新 `状态`。
>
> 未含无真实影响的 D 类误报（如 `json_output.rs` 花括号 69/67 与 HEAD 一致，为字符串内括号的存量假象）；agent 报告中的纯风格问题未收录。

## 一、缺陷统计

| 严重度 | 数量 | 说明 |
|---|---|---|
| 严重 (Critical) | 12 | 凭据泄露 / RCE / 数据不可逆丢失 / 全凭据失效 / OOM / 磁盘写满 |
| 高 (High) | 22 | 主线程冻结 / 数据丢失 / TOFU 信任接管 / 会话黑屏 |
| 中 (Medium) | 21 | 资源泄漏 / 竞态 / 会话挂起 / 内存滞留 |
| 低 (Low) | 3 | 轻微泄漏 / 掉帧 |

| 类别 | 数量 |
|---|---|
| A 并发/线程安全 | 13 |
| B 资源管理 | 7 |
| C 生命周期 | 7 |
| D 编译结构 | 3 |
| E 安全漏洞 | 21 |
| F 持久化一致性 | 7 |

> 注：部分条目跨类别，以上按主类别统计。

## 二、修复优先级（建议实施顺序）

1. **P0（凭据/执行边界，12 条 Critical）**：#25 #32 #33 #35 #39 #40 #45 #49 #53 #54 #55 #56
2. **P1（数据不丢/信任接管）**：#8 #13 #24 #27 #28 #36 #46 #47 #48 #50 #52 #57
3. **P2（UI 冻结/会话挂起）**：#1 #9 #16 #17 #18 #19 #20 #21 #23 #29 #42 #44
4. **P3（资源/竞态/zeroize/低危）**：#2 #3 #4 #5 #6 #7 #10 #11 #12 #14 #15 #22 #26 #30 #31 #34 #37 #38 #41 #43 #51 #58

## 三、缺陷明细

> `状态` 取值：`待修复` → `已修复` → `已验证(CI)`。修复时直接在本表改状态并附提交哈希。

### G1 — app.rs 主状态机（src/app.rs）

---

#### #1 — WebDAV 上传/下载阻塞 UI 线程
- **文件**: `src/app.rs`（1301-1341、1729-1776）
- **类别**: A 并发/线程安全　**严重度**: 高
- **问题**: `webdav_put_json`/`webdav_get_json` 直接在 Slint UI 回调里同步执行 blocking HTTP 与 JSON 导入导出。
- **场景**: 网络慢时整个事件循环冻结数秒，泵线程的 `invoke_from_event_loop` 同步被堵，所有会话输出停滞。
- **修复**: 移入 `runtime.spawn` 后台执行，完成后 `invoke_from_event_loop` 回写状态。
- **状态**: 已验证(CI) · fix(ui) c6efea0

#### #2 — 1Hz 系统采样与 MCP 活动轮询阻塞 UI 线程
- **文件**: `src/app.rs`（2616-2653、2446-2465）
- **类别**: A 并发/线程安全　**严重度**: 中
- **问题**: 系统采样（sysinfo）与 MCP 活动轮询（读 jsonl）在 UI 线程同步执行。
- **场景**: 窗口活跃或设置页打开时周期性执行，磁盘/sysinfo 慢时 UI 卡顿。
- **修复**: 采样移 `spawn_blocking`，结果缓存后刷新。
- **状态**: 已验证(CI) · fix(ui) c6efea0

#### #3 — 每次操作即全量 save() 写配置
- **文件**: `src/app.rs`（5371 `on_run_command`、`on_persist_sidebar_width` 等）
- **类别**: A 并发/线程安全　**严重度**: 低
- **问题**: 每次用户操作都全量重写配置文件，且落在 UI 线程。
- **场景**: 会话多、配置大时频繁整文件写造成掉帧。
- **修复**: 防抖/合并写盘，移出 UI 线程。
- **状态**: 已验证(CI) · fix(ui) c6efea0

#### #4 — 剪贴板线程无界累积
- **文件**: `src/app.rs`（795、5383、6137、6498、6520）
- **类别**: B 资源管理　**严重度**: 中
- **问题**: 每次复制起一个 `clipboard_set_text` 线程，`set().wait()` 阻塞至他程序接管；无剪贴板管理器时永不退出。
- **场景**: Linux 无剪贴板管理器环境反复复制 → 线程无限累积。
- **修复**: 单一后台线程 + 队列，或加超时。
- **状态**: 已验证(CI) · fix(ui) c6efea0

#### #5 — 关窗不取消在途会话/认证任务
- **文件**: `src/app.rs`（2100 `spawn_session`、4435/4479 `test_session_auth`）
- **类别**: C 生命周期　**严重度**: 中
- **问题**: 关标签/窗口仅 close handle，不取消共享 runtime 上在途任务。
- **场景**: 连接中关标签/窗口，握手任务残留至进程退出；快速开关堆积后台任务。
- **修复**: 记录 JoinHandle，关窗时 abort。
- **状态**: 已验证(CI) · fix(ui) c6efea0

#### #6 — 跨线程共享锁一律 unwrap()/expect()
- **文件**: `src/app.rs`（81、107、113、1477、2637）
- **类别**: D 编译结构　**严重度**: 中
- **问题**: `bufs`/`tab_statuses`/`sampler` 等共享 Mutex 全部 `lock().unwrap()`。
- **场景**: 任一线程持锁 panic 毒化锁，UI 线程下次访问直接 panic 崩掉整个应用。
- **修复**: 改 `if let Ok(...)` 降级跳过。
- **状态**: 已验证(CI) · fix(ui) c6efea0

#### #7 — 导出 JSON 用内置固定 key 混淆密码
- **文件**: `src/app.rs`（3835）
- **类别**: E 安全漏洞　**严重度**: 高
- **问题**: 导出的 sessions 文件密码用二进制内固定 key 混淆，可逆。
- **场景**: 导出的文件被他人获取即可轻易还原全部密码。
- **修复**: 用强加密并明示，或提示导出非加密。
- **状态**: 已验证(CI) · fix(ui) c6efea0

#### #8 — 写盘错误被静默吞掉
- **文件**: `src/app.rs`（971、1023、1294、2174 等大量 `let _ = s.save()`）
- **类别**: F 持久化一致性　**严重度**: 高
- **问题**: `let _ = s.save()` 丢弃写盘错误。
- **场景**: 磁盘满/权限错误时会话与配置改动静默丢失。
- **修复**: 至少 warn 日志并提示用户。
- **状态**: 已验证(CI) · fix(ui) c6efea0

---

### G2 — app 核心子模块（src/app/*）

#### #9 — wsl_available() 阻塞 UI 线程
- **文件**: `src/app/session_models.rs:386`
- **类别**: A 并发/线程安全　**严重度**: 高
- **问题**: `wsl_available()` 在 UI 线程同步执行 `wsl.exe --status`，无超时。
- **场景**: 首次同步会话列表时若 wsl.exe 卡住（首次初始化），主线程阻塞、界面冻结。
- **修复**: 结果改后台 tokio 任务探测并加超时，`OnceLock` 只存缓存 bool。
- **状态**: 已验证(CI) · fix(ui) c6efea0

#### #10 — 无界 mpsc 事件通道
- **文件**: `src/app/session_runtime.rs:83`
- **类别**: B 资源管理　**严重度**: 中
- **问题**: 会话与 SFTP 事件通道均为无界 mpsc；批次含立即 UI 事件时跳过背压等待。
- **场景**: `tail -f` 高吞吐叠加 1Hz 状态事件时，通道与终端缓冲无界增长，内存膨胀。
- **修复**: 换有界通道，或持续过载时合并/丢帧限流。
- **状态**: 已验证(CI) · fix(ui) c6efea0

#### #11 — SFTP 传输列表无限累积
- **文件**: `src/app/session_event.rs:354`
- **类别**: B 资源管理　**严重度**: 中
- **问题**: `SftpTransfer` 完成/失败行从不移除，只 insert/set。
- **场景**: 长会话大量传输后列表无限累积，每次事件 O(n) 扫描，UI 变慢。
- **修复**: 结束态停留一段时间后删除，或设最大条数。
- **状态**: 已验证(CI) · fix(ui) c6efea0

#### #12 — 单实例 IPC 无认证无权限保护
- **文件**: `src/app/single_instance.rs:102`
- **类别**: E 安全漏洞　**严重度**: 中
- **问题**: IPC 端口文件无权限保护、TCP/Unix socket 无认证，任意本地进程可写/连接。
- **场景**: 本地进程覆盖 `ipc.port` 指向伪造监听者回 ack 吞掉 `--new-window`；或向 primary 刷弹窗；读改写竞态还有双 Primary 窗口期。
- **修复**: 端口文件/socket 设 0600，握手加随机 token 校验。
- **状态**: 已验证(CI) · fix(ui) c6efea0

#### #13 — 触发器响应按下标关联错配
- **文件**: `src/app/session_trigger.rs:28`
- **类别**: F 持久化一致性　**严重度**: 高
- **问题**: 已存 Secret 回复按下标 `saved_responses.get(index)` 关联，draft 无稳定 id。
- **场景**: 编辑会话时删除/重排触发器，回复错配到其他 expect，向远端发送错误口令。
- **修复**: `TriggerDraft` 携带原索引/id 定位，而非当下下标。
- **状态**: 已验证(CI) · fix(ui) c6efea0

#### #14 — SessionHandle 未随 TabRoute 迁移
- **文件**: `src/app/session_runtime.rs:58`
- **类别**: A 并发/线程安全　**严重度**: 中
- **问题**: `SessionHandle` 存源窗口 `ctx.handles` 未随 TabRoute 路由，而 SFTP handle 却走 route，不对称。
- **场景**: 连接建立中拖拽 tab 换窗，新窗口可渲染事件但按 id 找不到 handle，键入输入丢失。
- **修复**: handle 同样经 route 发布，或 detach 时同步迁移 handles 条目。
- **状态**: 跳过（G2：tab_transfer.rs 已在 move_tab_between_windows 迁移 handles，路由重写与 handle 迁移在 start/close/move 路径成对；架构性改造需改 core.rs，超出本组范围）

#### #15 — UI 回调裸 panic 点
- **文件**: `src/app/session_event.rs:20`
- **类别**: D 编译结构　**严重度**: 中
- **问题**: 模型 downcast 用 `expect`、`statuses.lock().unwrap()` 等裸恐慌点。
- **场景**: `.slint` 结构变动或某后台线程持锁 panic 致锁中毒时，UI 线程回调 panic，整进程崩溃。
- **修复**: 改 `if-let`/`ok()` 降级跳过。
- **状态**: 已验证(CI) · fix(ui) c6efea0

---

### G3 — app UI 回调层（src/app/*）

#### #16 — 预设下载目录路径阻塞 UI 线程
- **文件**: `src/app/sftp_callbacks.rs:136`
- **类别**: A 并发/线程安全　**严重度**: 高
- **问题**: 设置固定下载目录后 `h.download`/`download_archive` 在 UI 线程同步执行（同函数另一分支却走 `thread::spawn`）。
- **场景**: 大文件传输在主线程阻塞，界面完全冻结。
- **修复**: 与 picker 分支一致，冲突确认+传输整体移入后台线程，`upgrade_in_event_loop` 回写。
- **状态**: 已验证(CI) · fix(ui) c6efea0

#### #17 — 后台传输期间跨线程持有 sftp_handles 锁
- **文件**: `src/app/sftp_callbacks.rs:225`（另见 L160-170）
- **类别**: A 并发/线程安全　**严重度**: 高
- **问题**: 上传/下载后台线程在整段传输期间持有 `sftp_handles` 互斥锁。
- **场景**: 多会话镜像上传时锁被长占，UI 线程 navigate/refresh/delete/关标签回调全部阻塞在 `lock()` 上，界面假死。
- **修复**: 锁内先 clone 出 `SftpHandle`（Rc）再释放锁，锁外执行传输。
- **状态**: 已验证(CI) · fix(ui) c6efea0

#### #18 — 远端到远端复制阻塞 UI 线程
- **文件**: `src/app/sftp_callbacks.rs:522`
- **类别**: A 并发/线程安全　**严重度**: 高
- **问题**: `copy_to`（本地临时目录中转 + 双端网络 IO）在 UI 线程同步执行。
- **场景**: 跨会话复制大文件时 UI 冻结。
- **修复**: 复制操作移入后台线程。
- **状态**: 误报（copy_to 实为 commands.send(SftpCommand::CopyTo) 通道消息，实际复制在每会话 SFTP worker 任务执行，非阻塞，不冻结 UI）

#### #19 — SFTP 网络往返全部同步执行
- **文件**: `src/app/sftp_callbacks.rs:80`
- **类别**: A 并发/线程安全　**严重度**: 高
- **问题**: `list_dir`/`refresh_dir`/`delete`/`read_text` 等 SFTP 网络往返直接在回调里同步执行。
- **场景**: 高延迟网络下列目录、删除、打开远程大文件均阻塞 UI。
- **修复**: 统一提交到每会话工作线程，完成后经事件循环回写模型。
- **状态**: 误报（list_dir/refresh_dir/delete/read_text 均为通道 send，网络往返在 run_sftp worker，UI 线程仅发消息，不阻塞）

#### #20 — 关标签页同步 close 会话
- **文件**: `src/app/tab_callbacks.rs:155`
- **类别**: C 生命周期　**严重度**: 中
- **问题**: 关标签页在 UI 线程同步 `close()` 会话并 `lock().unwrap()`。
- **场景**: 该标签有进行中传输时锁被占，close 阻塞至传输结束才关闭；后台线程 panic 弄脏锁则 `.unwrap()` 让 UI 线程 panic。
- **修复**: 关闭移出 UI 线程；改用 `unwrap_or_else` 处理毒锁。
- **状态**: 已验证(CI) · fix(ui) c6efea0

#### #21 — 拖放清空源窗口时同步 teardown
- **文件**: `src/app/tab_transfer.rs:480`
- **类别**: C 生命周期　**严重度**: 中
- **问题**: 拖放清空源窗口时在拖放回调里同步 `teardown_window`（关会话/join 线程）。
- **场景**: 大传输中拖走唯一标签，源窗口关闭阻塞拖放回调。
- **修复**: teardown 异步化，先关 UI 再释放句柄。
- **状态**: 跳过（G3：teardown_window 只发 Close 非阻塞消息并清表，不 join 线程；阻塞的 save_layout/clear_zen_on_close 在 app.rs 且状态为 Rc<RefCell>（非 Send），无法从 tab_transfer.rs 移出）

#### #22 — 壁纸加载阻塞 UI 线程
- **文件**: `src/app/terminal_ui.rs:306`
- **类别**: A 并发/线程安全　**严重度**: 低
- **问题**: 壁纸 `load(id)` 在 UI 线程读盘+解码。
- **场景**: 切换大尺寸壁纸时界面明显卡顿。
- **修复**: 后台线程解码，完成后事件循环设属性。
- **状态**: 跳过（G3：wallpaper::load 产出含 slint::Image 的 Wallpaper 非 Send，移后台需重构 wallpaper 模块，超出允许编辑范围，低危）

#### #23 — WebDAV Basic 密码明文 + 同步阻塞 HTTP
- **文件**: `src/app/webdav.rs:74`
- **类别**: E 安全漏洞　**严重度**: 高
- **问题**: Basic 认证密码允许经明文 `http://` 传输；`webdav_put/get_json` 为同步阻塞 HTTP（20s 超时）。
- **场景**: 用户配 `http://NAS:5005` 时凭据可被 LAN 嗅探；若由 UI 回调直接调用，不可达时冻结最长 20s。
- **修复**: 默认禁 http 仅显式放行；调用方包 `thread::spawn`。
- **状态**: 已验证(CI) · fix(ui) c6efea0

#### #24 — 「记住密码」同步读写 + 明文滞留
- **文件**: `src/app/auth_dialogs.rs:307`
- **类别**: F 持久化一致性　**严重度**: 高
- **问题**: 「记住密码」在 UI 线程同步读-改-写配置文件，且 `CRED_DECIDED` 全程保留明文密码。
- **场景**: 快速连续连接同会话时写盘交错，配置可能丢失；密码滞留内存并在重连时自动复用。
- **修复**: 写盘改原子替换（临时文件+rename）并异步化；缓存不存明文密码。
- **状态**: 已验证(CI) · fix(ui) c6efea0

---

### G4 — SSH 连接与凭据（src/ssh/*）

#### #25 — https 代理协议降级为明文 CONNECT
- **文件**: `src/ssh/impls/proxy.rs:104`
- **类别**: E 安全漏洞　**严重度**: 严重
- **问题**: `https` 代理协议被当作 `http` 同等处理，CONNECT 请求（含 `Proxy-Authorization: Basic` 代理凭据）以明文 TCP 发送。
- **场景**: 用户配置 `https://user:pass@proxy` 期望 TLS 保护，实际凭据明文过网，路径第三方可嗅探窃取代理口令。
- **修复**: 对 `https` 协议先用 rustls/native-tls 包裹 TcpStream 再发 CONNECT；不支持 TLS 则拒绝该 scheme 而非静默降级。
- **状态**: 已验证(CI) · fix(ssh) 8cd6a1f

#### #26 — 代理 Basic 认证 token 未 zeroize
- **文件**: `src/ssh/impls/proxy.rs:111`
- **类别**: E 安全漏洞　**严重度**: 中
- **问题**: 代理 Basic 认证 token 与 CONNECT 请求串为普通 `String`，未 zeroize。
- **场景**: 进程内存被转储时，`base64(user:pass)` 残留在堆上。
- **修复**: 用 `Zeroizing<String>`/`Secret` 承载 token 与请求串，发送后显式清零。
- **状态**: 已验证(CI) · fix(ssh) 8cd6a1f

#### #27 — known_hosts 读改写竞态 + 非原子写
- **文件**: `src/ssh/impls/known_hosts.rs:88`
- **类别**: F 持久化一致性　**严重度**: 高
- **问题**: `remember()` 是「读-改-写全量重写」，无锁且 `std::fs::write` 非原子。
- **场景**: 多个会话同时完成首次主机密钥确认时，后写者覆盖先写者刚追加的条目；写盘中途崩溃损坏整个 known_hosts。
- **修复**: 进程内互斥锁串行化写操作；写临时文件后 `rename` 原子替换。
- **状态**: 已验证(CI) · fix(ssh) 8cd6a1f

#### #28 — known_hosts 无权限/软链防护
- **文件**: `src/ssh/impls/known_hosts.rs:28`
- **类别**: E 安全漏洞　**严重度**: 高
- **问题**: known_hosts 路径无权限/属主校验、无符号链接防护，`remember` 跟随软链写入。
- **场景**: 攻击者预置软链或预先写入自己的密钥行，使目标主机被静默 `Match`，TOFU 信任被接管（首连提示形同虚设）。
- **修复**: 写入前检查父目录/文件属主与权限（0600），拒绝符号链接目标。
- **状态**: 已验证(CI) · fix(ssh) 8cd6a1f

#### #29 — suppress_echo 无超时释放
- **文件**: `src/ssh/impls/ssh.rs:2003`
- **类别**: C 生命周期　**严重度**: 高
- **问题**: `suppress_echo` 无超时释放；prompt setup 写入失败或远程 eval 未执行时 OSC699 完成标记永不出现。
- **场景**: `channel.data(prompt_setup)` 失败、或 bash/zsh 未真正执行该命令时，标记不返回，此后所有终端输出被 `echo_buf` 吞掉且不显示，会话全程黑屏（`bound_prompt_setup_echo` 只限缓冲大小，不恢复显示）。
- **修复**: 为 `suppress_echo` 增加超时/累计字节数上限，超时后冲刷缓冲、清除标志恢复正常渲染。
- **状态**: 已验证(CI) · fix(ssh) 8cd6a1f

#### #30 — 密码认证路径未 zeroize
- **文件**: `src/ssh/impls/ssh.rs`（1100 `resolve_credentials`、2707 `keyboard_interactive_auth`）
- **类别**: E 安全漏洞　**严重度**: 中
- **问题**: `Secret` 在认证路径被复制为普通 `String`；`keyboard_interactive_auth` 把口令放入 `Vec<String>` 且未 zeroize。
- **场景**: `Secret` 只在 config 层生效，认证层全部退化为普通堆字符串；内存转储/日志泄漏风险。
- **修复**: 认证全程使用 `Secret`/`Zeroizing<String>`，`responses` 发送后显式清零。
- **状态**: 已验证(CI) · fix(ssh) 8cd6a1f

#### #31 — PPK Argon2 参数上限过宽
- **文件**: `src/ssh/impls/ppk.rs:22`
- **类别**: E 安全漏洞　**严重度**: 中
- **问题**: Argon2 参数上限过宽（内存 1 GiB、passes 1000、并行 64）。
- **场景**: 恶意/损坏的 PPK（如邮件附件粘贴）使客户端解析时分配近 1 GiB 内存并长时间占用 CPU，构成本地 DoS。
- **修复**: 收紧到实用上限（如 256 MiB / 32 passes / 4 并行）或按策略覆盖。
- **状态**: 已验证(CI) · fix(ssh) 8cd6a1f

---

### G5 — SFTP/隧道/WebDAV（src/sftp/*、src/tunnel/*、src/webdav/*）

#### #32 — WebDAV 证书校验 fail-open
- **文件**: `src/webdav/impls/certificate_verifier.rs:4`
- **类别**: E 安全漏洞　**严重度**: 严重
- **问题**: 证书校验完全 fail-open，`verify_server_cert` 无条件返回 Ok，任何证书都放行。
- **场景**: 若该 verifier 是 WebDAV 唯一校验路径，TLS 层可被中间人，账号口令可被窃取。
- **修复**: 仅当用户显式勾选「信任任意证书」时注入此 verifier；默认路径改用系统信任库 + ServerName 主机名校验。
- **状态**: 已验证(CI) · fix(sftp) 2d645a7

#### #33 — SFTP「外部打开」可执行远程文件（RCE 链）
- **文件**: `src/sftp/impls/sftp.rs:1240`（另见 1516-1548）
- **类别**: E 安全漏洞　**严重度**: 严重
- **问题**: 「外部打开」用远程可控文件名落盘并交给 OS 执行：`OpenTemp` 把远程 basename 原样（sanitize 只改分隔符）下载到 `%TEMP%\meatshell\...` 后 `ShellExecuteW("open")` 直接运行。
- **场景**: 恶意服务器提供 `foo.exe`/`.bat`/`.scr` 等文件，用户一次点击即在本机执行服务器内容。
- **修复**: 对可执行扩展名拒绝「打开」或强制改名/只读查看；至少弹高危确认。
- **状态**: 已验证(CI) · fix(sftp) 2d645a7

#### #34 — SFTP 远端路径未消毒可越目录操作
- **文件**: `src/sftp/impls/sftp.rs:1760`（另见 1053-1113）
- **类别**: E 安全漏洞　**严重度**: 中
- **问题**: 列表 `full_path` 用远程不可信 name 直接拼接，删除/重命名路径未再消毒。
- **场景**: 恶意服务器返回含 `/` 或 `..` 的条目名，构造出目录外路径；用户点删除/重命名会作用于目录外远端文件（本地下载侧有 sanitize 防护，仅远端操作受影响）。
- **修复**: `read_dir` 后校验 name 不含 `/` `\` 且不为 `..`，否则拒绝/替换。
- **状态**: 已验证(CI) · fix(sftp) 2d645a7

#### #35 — 上传/文本保存非原子（TRUNCATE 覆盖）
- **文件**: `src/sftp/impls/sftp.rs:2161`（另见 2164、2227/2232、1436-1448）
- **类别**: F 持久化一致性　**严重度**: 严重
- **问题**: 上传/文本保存均为「先 TRUNCATE 再流式写」非原子；`CREATE|WRITE|TRUNCATE` 打开，中途断线即 `raw.remove(remote)` 删除整个旧文件；`write_text_file` 中途失败留半截。
- **场景**: 重传已存在的重要远端文件在弱网下会永久丢失旧版本。
- **修复**: 先写远端临时文件（`remote+.tmp-uuid`），成功后 `sftp.rename` 覆盖目标；失败仅删临时。
- **状态**: 已验证(CI) · fix(sftp) 2d645a7

#### #36 — 会话关闭不取消在途传输
- **文件**: `src/sftp/impls/sftp.rs:604`（另见 623/748/846、1327）
- **类别**: C 生命周期　**严重度**: 高
- **问题**: 会话关闭不取消在途传输，任务被 detach 且无 JoinHandle 追踪。
- **场景**: Close 只断开连接，已 spawn 的下载/上传任务继续运行或直到 keepalive 超时（~90s）才报错退出；期间半成品文件残留、cancel 表项滞留，面板已关用户无法取消。
- **修复**: 维护在途任务集合，Close 时统一置 cancel 标志并 await/abort 所有 JoinHandle。
- **状态**: 已验证(CI) · fix(sftp) 2d645a7

#### #37 — 空文件夹下载漏删 cancel 表项
- **文件**: `src/sftp/impls/sftp.rs:639`
- **类别**: A 并发/线程安全　**严重度**: 低
- **问题**: 空文件夹下载提前 `return` 时未执行 `cancels_done.remove`。
- **场景**: `is_dir` 且为空时提前返回路径漏删条目，`Mutex<HashMap>` 条目泄漏，反复操作内存缓慢增长；对应 `CancelTransfer` 置位无人消费。
- **修复**: 提前返回路径也执行 remove；或统一在任务末尾清理。
- **状态**: 已验证(CI) · fix(sftp) 2d645a7

#### #38 — SOCKS5 无认证开放代理风险
- **文件**: `src/tunnel/impls/forward.rs:26`（另见 155）
- **类别**: E 安全漏洞　**严重度**: 高
- **问题**: SOCKS5 无认证（应答 METHOD=0），绑非回环地址即成局域网开放代理。
- **场景**: 空 bind 默认 127.0.0.1 安全，但 UI 允许用户填 0.0.0.0 且无警告；此时 -D 是任意局域网主机可用的免认证代理直通 SSH 内网，-L 也把本地端口暴露给局域网。
- **修复**: 检测非回环绑定地址时强制弹窗确认，并对 -D 提示「无认证开放代理」风险。
- **状态**: 已验证(CI) · fix(sftp) 2d645a7

---

### G6 — 终端渲染与协议（src/terminal/*）

#### #39 — ZMODEM 数据子包无上限累积（OOM）
- **文件**: `src/terminal/impls/zmodem.rs:365`（read_subpacket）
- **类别**: B 资源管理　**严重度**: 严重
- **问题**: 数据子包在内存中无上限累积，且 CRC 校验前整包 `data.clone()` 使峰值内存翻倍。
- **场景**: 恶意/损坏对端在 ZDATA 中不发送 ZDLE(0x18) 终止符，`data` Vec 随通道数据无限增长直至 OOM；ZEOF/ZRPOS 皆无法到达。
- **修复**: 按声明文件剩余大小或固定上限（如 1MiB）截断子包，改用流式增量 CRC，避免整包缓冲+克隆。
- **状态**: 已验证(CI) · fix(terminal) 1bcef38

#### #40 — ZMODEM 写盘不校验声明大小（磁盘写满）
- **文件**: `src/terminal/impls/zmodem.rs:132`（ZDATA 写盘循环）
- **类别**: B 资源管理　**严重度**: 严重
- **问题**: 不校验 `c.written` 是否超过声明 `size`。
- **场景**: 对端声明小 size（parse 失败时为 0）却持续流式发送，无限写盘 → 用户磁盘被写满（Disk-fill DoS），且文件数量无上限。
- **修复**: 写入前检查 `written+len > size` 即中止；限制单会话文件数与总字节。
- **状态**: 已验证(CI) · fix(terminal) 1bcef38

#### #41 — ZMODEM 传输失败残留半截文件
- **文件**: `src/terminal/impls/zmodem.rs:118`（ZFILE 建文件）
- **类别**: B 资源管理　**严重度**: 中
- **问题**: 传输失败残留半截文件。
- **场景**: CRC 错、30s 超时、对端 ZCAN/ZABORT 任一 bail 路径都不删除已建文件，Downloads 持续累积垃圾并占盘。
- **修复**: 错误路径删除未完成文件（如 `remove_file`）。
- **状态**: 已验证(CI) · fix(terminal) 1bcef38

#### #42 — 串口写超时后句柄泄漏
- **文件**: `src/terminal/impls/serial.rs:195`
- **类别**: B 资源管理　**严重度**: 高
- **问题**: `timeout(30s, spawn_blocking(write_all))` 超时后 spawn_blocking 任务不可取消，仍占住阻塞线程池并持有串口写句柄克隆。
- **场景**: 硬件流控对端停流时 write_all 永久阻塞；30s 后会话关闭、reader 退出，但串口设备因泄漏句柄不被释放，Windows 下重开报「端口被占用」。
- **修复**: 串口设写超时，或改用一个可由 Close 信号唤醒的写线程并在退出时 join、回收句柄。
- **状态**: 已验证(CI) · fix(terminal) 1bcef38

#### #43 — local reader 线程 detach 不 join
- **文件**: `src/terminal/impls/local.rs:111`
- **类别**: C 生命周期　**严重度**: 中
- **问题**: reader 线程被 detach，Close 分支 kill 子进程后直接 break，从不 join。
- **场景**: 若 kill 未即时关闭 PTY 读端，reader 线程连同 master 读句柄泄漏；线程内克隆的 `reader_events` 也延迟释放。
- **修复**: 保存 JoinHandle，退出前主动关闭读端并 join。
- **状态**: 已验证(CI) · fix(terminal) 1bcef38

#### #44 — Telnet IAC 子协商状态可被悬挂
- **文件**: `src/terminal/impls/telnet.rs:253`（Sub 状态）
- **类别**: D 编译结构　**严重度**: 中
- **问题**: 对端发 `IAC SB` 后不发 `IAC SE`，解析器永久停在子协商状态，丢弃之后所有数据字节。
- **场景**: 异常/恶意 Telnet 服务器开启子协商后不结束 → 终端输出静默丢失、界面冻结（无超时、无上限）。
- **修复**: 子协商加长度/时间上限，超限强制回到 Data 态。
- **状态**: 已验证(CI) · fix(terminal) 1bcef38

---

### G7 — 配置与凭据持久化（src/config/*、src/session/*）

#### #45 — secret.key 损坏时静默重生成丢全部凭据
- **文件**: `src/config/impls/config.rs:423`
- **类别**: E 安全漏洞 / D 编译结构　**严重度**: 严重
- **问题**: `secret.key` 长度非 32 时静默重新生成新 key，旧 key 既不备份也不告警；所有已存 `enc:v1:` 密文用新 key 解密全部失败，加载后密文留在 cache，保存时因带前缀跳过重加密——用户全部密码静默永久失效。
- **场景**: secret.key 被同步工具/磁盘错误截断，或首写中断。
- **修复**: 重生成前把旧 key rename 为 `secret.key.broken`；加载时若「存在 enc:v1 但全部解密失败」应告警并引导重新录入。
- **状态**: 已验证(CI) · fix(config) ce5f14f

#### #46 — 新 key write-then-chmod 竞态
- **文件**: `src/config/impls/config.rs:436`
- **类别**: E 安全漏洞　**严重度**: 高
- **问题**: 新 key 先 `fs::write` 再 `set_permissions(0600)`，窗口期内按 umask（通常 0644）可被其他本地用户读取；若进程在此中断，key 永久 world-readable，且已存在（32 字节）的 key 之后不再重新 chmod。
- **场景**: 首次启动/重生成 key 时，其他本地用户可在窗口期读到密钥。
- **修复**: 用 `OpenOptions`+`mode(0600)` 创建即锁权，或加载路径对已存在 key 也强制 chmod。
- **状态**: 已验证(CI) · fix(config) ce5f14f

#### #47 — 固定 tmp 文件名多实例并发覆盖
- **文件**: `src/config/impls/config.rs:1585`
- **类别**: A 并发/线程安全 / F 持久化一致性　**严重度**: 高
- **问题**: 临时文件路径固定为 `sessions.json.tmp`，且 ConfigStore 无跨实例协调（各窗口/各实例独立持有 cache）。
- **场景**: 多窗口同时编辑保存；或双开应用 → 多实例并发保存写同一 tmp 再 rename，互相覆盖（丢失更新）甚至交错写坏文件。
- **修复**: tmp 用 pid/uuid 唯一名，保存前重读磁盘做 merge，或引入单例/文件锁。
- **状态**: 已验证(CI) · fix(config) ce5f14f

#### #48 — 配置读→改→写非原子，崩溃丢失
- **文件**: `src/config/impls/config.rs:1549`
- **类别**: C 生命周期 / F 持久化一致性　**严重度**: 高
- **问题**: 所有 setter/upsert/remove/reorder 只改内存 cache，save() 由调用方择机触发；崩溃/强杀丢失自上次保存以来的全部改动，未见退出统一保存钩子。
- **场景**: 用户改完配置后应用崩溃/强杀 → 改动全部丢失。
- **修复**: 变更即防抖落盘，或退出时统一保存 + 原子 rename。
- **状态**: 已验证(CI) · fix(config) ce5f14f

#### #49 — 明文密码前缀误判跳过加密
- **文件**: `src/config/impls/config.rs:1553`
- **类别**: E 安全漏洞　**严重度**: 严重
- **问题**: 明文密码恰以 `enc:v1:` 开头时 save 跳过加密，明文落盘（password/private_key_inline/webdav_password/trigger.response 同）。
- **场景**: 用户密码字面量以 `enc:v1:` 开头 → 明文写入配置文件。
- **修复**: 以「try_decrypt 成功」为密文判据，而非前缀字符串匹配。
- **状态**: 已验证(CI) · fix(config) ce5f14f

#### #50 — 迁移后旧明文文件残留
- **文件**: `src/config/impls/config.rs:137`
- **类别**: E 安全漏洞　**严重度**: 高
- **问题**: `migrate_legacy` 仅复制不删除旧目录 sessions.json；升级前若含明文密码，明文永久残留 legacy 目录（`.json.broken` 同理不清理）。
- **场景**: 升级到加密版本后，旧明文会话文件仍留在磁盘上可被读取。
- **修复**: 迁移成功后提示并安全清理旧明文文件。
- **状态**: 已验证(CI) · fix(config) ce5f14f

#### #51 — FinalShell 解密明文未 zeroize
- **文件**: `src/config/impls/finalshell.rs:141`
- **类别**: E 安全漏洞　**严重度**: 中
- **问题**: `decode_password` 解密出的 plaintext Vec、key_material Vec 与 8 字节 DES key 均未 zeroize。
- **场景**: 明文与 key 材料残留在堆内存，进程内存被转储时泄漏。
- **修复**: 用 zeroize 容器存放并在使用后清零。
- **状态**: 已验证(CI) · fix(config) ce5f14f

#### #52 — 备份 key 非原子 + tmp 残留
- **文件**: `src/config/impls/config.rs:1603 / 1586`
- **类别**: F 持久化一致性 / B 资源管理　**严重度**: 中
- **问题**: 备份先 rename sessions.json 再对 secret.key 用非原子 `fs::copy`，中断产生「新密文+旧/半写 key」的不一致备份；且 save 出错/崩溃时 `.json.tmp` 残留不清理。
- **场景**: 备份过程崩溃 → 恢复备份时用旧 key 解新密文，全部失败；tmp 垃圾累积。
- **修复**: key 也走 tmp+rename；出错时删除 tmp。
- **状态**: 已验证(CI) · fix(config) ce5f14f

---

### G8 — MCP/自动化/CLI/日志/杂项（src/mcp/*、src/automation/*、src/cli/*、src/logging/*）

#### #53 — MCP download_file 路径遍历本地任意写
- **文件**: `src/automation/impls/tools.rs:68`
- **类别**: E 安全漏洞　**严重度**: 严重
- **问题**: `download_file` 把 remote_path 末段直接 join 到 local_directory，未过滤 `\` 与 `..`；且先 exists() 检查却用 `DownloadConflict::Replace`，存在 TOCTOU 覆盖。
- **场景**: 攻击者先 run_command 在远端造一个名为 `..\..\evil` 的文件，再 download_file 指定任意已存在 local_directory；win32 下 `Path::join` 会把 `..\` 段解析到目录之外，实现本地任意写。
- **修复**: 用 `rsplit(['/', '\\']).next()` 取末段，拒绝含 `..`/绝对路径/空白的文件名；落盘前规范化并断言在 local_directory 内；冲突策略与「不覆盖」承诺一致（改拒绝）。
- **状态**: 已验证(CI) · fix(mcp) f2669de

#### #54 — MCP upload_file 无沙箱任意本地文件外传
- **文件**: `src/automation/impls/tools.rs:33`
- **类别**: E 安全漏洞　**严重度**: 严重
- **问题**: `upload_file` 的 local_path 无沙箱/白名单，MCP 可上传任意本地文件。
- **场景**: 只要 `mcp_allow_file_transfers` 开启，被 prompt 注入的 AI 或恶意客户端即可把 `~/.ssh/id_rsa`、error.log 等上传到已保存会话的远端目录，构成本地任意文件读取+外传。
- **修复**: 限定 local_path 必须位于用户配置的传输目录内（或路径允许列表），并记录每次传输源目标。
- **状态**: 已验证(CI) · fix(mcp) f2669de

#### #55 — MCP 单一命令开关授予全部会话 RCE
- **文件**: `src/automation/impls/tools.rs:238`
- **类别**: E 安全漏洞　**严重度**: 严重
- **问题**: `mcp_allow_commands` 单一布尔开关即授予对全部已保存 SSH 会话的任意命令执行，无命令/会话白名单、无调用方鉴权。
- **场景**: 开关一开，任何能拉起 `meatshell mcp serve` 的本地进程即对全部保存会话拥有 RCE+凭据能力；caller 仅取自客户端自报的 initialize 信息。
- **修复**: 增加会话级+命令前缀白名单，高危操作要求用户确认，或为 MCP 引入独立令牌。
- **状态**: 已验证(CI) · fix(mcp) f2669de

#### #56 — MCP run_command 明文审计落盘泄漏内联密码
- **文件**: `src/mcp/impls/server.rs:140`
- **类别**: E 安全漏洞　**严重度**: 严重
- **问题**: 审计白名单含 command，run_command 全量明文写入 `mcp_activity.jsonl`（start/end 各一次）。
- **场景**: 命令内联密码/token（`curl -H "Authorization: Bearer …"`、`echo pw | sudo -S`）会被明文落盘到共享审计文件，任意本地进程可读，违背「审计绝不明文写 secrets」约束。
- **修复**: 对 command 脱敏（正则替换常见 secret 模式）或默认仅记 tool+hash、命令记录改为显式开关。
- **状态**: 已验证(CI) · fix(mcp) f2669de

#### #57 — MCP 活动审计 trim/clear 非原子跨进程无锁
- **文件**: `src/mcp/impls/activity.rs:63`
- **类别**: F 持久化一致性　**严重度**: 高
- **问题**: 超限 trim 用 `std::fs::write` 截断重写，`clear_activity` 直接截断，且进程间（MCP vs GUI）无锁。
- **场景**: GUI 另一进程 tail 与 MCP append 并发，trim 时读到空/半文件而跳过记录；双 MCP 进程并发 trim 互相覆盖，审计记录丢失。
- **修复**: trim 改「写临时文件+原子 rename」；clear 改 rename 旧文件后新建，保证读者不遇半文件。
- **状态**: 已验证(CI) · fix(mcp) f2669de

#### #58 — MCP 单循环串行处理，慢操作阻塞全部
- **文件**: `src/mcp/impls/server.rs:36`
- **类别**: A 并发/线程安全　**严重度**: 中
- **问题**: 单循环 `block_on` 串行处理请求，一次最长 300s 的 SFTP/命令调用阻塞全部后续请求。
- **场景**: 恶意客户端发一个慢操作即可让 MCP 服务器停摆最长 300s，活动面板与其余工具全部不可用。
- **修复**: 每请求 spawn 独立任务并各自写 stdout（保持行序），或设更短的全局处理上限。
- **状态**: 已验证(CI) · fix(mcp) f2669de

---

## 四、无缺陷/健康模块（记录备查）

- `src/app/launch.rs`：纯 argv 解析，无状态/IO/并发，健康。
- `src/app/sidebar.rs`、`src/app/resource_ui.rs`、`src/app/jump_list.rs`：COM RAII 释放正确。
- `src/app/core.rs`、`src/app/window.rs`、`src/app/port_forward.rs`、`src/app/quick_commands.rs`：结构基本健康（`broadcast_config_changed` 存在 RefCell 重入理论风险，未确证）。
- PPK MAC 顺序：先解密后验 MAC 与 PuTTY 格式一致（MAC 覆盖明文），不算缺陷。
- host key 校验整体 fail-closed，`execute_command` 对未知主机正确拒绝。
- 终端滚动历史与 raw 流有 `MAX_HISTORY`(10 万行)/`RAW_CAP`(2MB) 上限，非无界；对端输入的 unwrap/panic 未发现（CRC 失配均 bail）；高亮 regex 线性引擎无 ReDoS。
- 终端滚动单行内存随屏宽线性、多会话叠加可达数百 MB（中风险，未列为缺陷）。
- ChaCha20-Poly1305 nonce 处理正确（每次 OsRng 随机 96-bit 且随密文持久化），Aead 出错不泄漏 key。

## 五、修复执行规则（对照 verified-methodology.md）

- 修复只改缺陷本身，不顺手重构；禁写「超级文件」。
- 按缺陷类型合理分组、一次修一批，不碎片化提交；提交用 Conventional Commits + GPG 签名（`git commit -S`）。
- 显式 `git add <file...>`，禁用 `git add -A`；push 用 `GIT_TERMINAL_PROMPT=0`。
- 禁止本地构建/测试；CI（GitHub Actions）是唯一裁判，多轮 push + `gh run view --log-failed` 迭代至全绿。
- 每条修复完成后回到本表更新 `状态` 并附提交哈希。
