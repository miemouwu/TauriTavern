# Issue: Windowed-payload "Cursor signature mismatch" on opening old chats (backfill fails → AI loses recent history; delete-floor errors)

> 交接文档。供另一个 agent 接手调查。包含**已核实**的代码定位、分优先级假设、决定性测试、调查步骤、候选修法。**已核实**项标 ✅;**假设/待证**标 ❓。

## 1. 现象(用户报告 + 截图)

打开一个**旧对话**,输入并收到回复后:

1. AI 回复"像是没有最近的回复历史,直接按历史记忆(角色卡/世界书)开始写"——即 prompt 缺最近对话楼。
2. 前端 toast:**`Context backfill failed [object Object]. Reload the chat to resync.`**
3. 后端错误:**`Failed to get chat payload before pages 五年之后1/五年之后 - 2026-06-12@13h23m26s: Validation error: Cursor signature mismatch for "/storage/emulated/0/Android/data/com.tauritavern.client/data/default-user/chats/五年之后1/...jsonl"`**
4. **删除楼层会报错**(同一个 cursor signature mismatch,删楼走窗口化保存校验)。

## 2. 环境(关键约束)

- 平台:**Android**,聊天文件在**外置/模拟存储** `/storage/emulated/0/Android/data/...`。
- 版本:**原版(vanilla/上游 release),不是带 Agent Memory P0 的分支** —— ⚠️ **不要把原因归到 P0 的 `stampAllMessages`(`getChatResult` 那段)**,用户的构建里没有这段。已排除。
- 用户装了一个**自己适配 TT 的「小白X」记忆/数据库插件**(第三方,**不在本仓库**),会**异步读取聊天历史**。

## 3. 机制(✅ 已核实代码)

窗口化载荷(windowed payload):前端内存 `chat[]` 只保留尾部窗口,完整历史在磁盘 JSONL,需要时**回填(backfill)**。回填/保存用一个 **cursor** 锚定文件位置,cursor 带**文件签名 = (size, modified_millis)**。

- ✅ **签名校验**:`src-tauri/src/infrastructure/repositories/file_chat_repository/windowed_payload_io.rs:256-267` `verify_cursor_signature`:
  ```rust
  if cursor.size != size || cursor.modified_millis != modified_millis {
      return Err("Cursor signature mismatch ...");
  }
  ```
- ✅ **签名来源**:同文件 `:54` `file_signature_from_metadata` = `(metadata.len(), 文件 mtime 毫秒)`。
- ✅ **任何让文件 size 或 mtime 变化的事(尤其一次保存)都会让旧 cursor 失效** → 用旧 cursor 回填/保存即报 mismatch。
- ✅ **保存串行、读不与写串行**:`src/script.js:563-596` `chatSaveQueue`/`enqueueChatSave`/`pendingChatSaveTasks` **只串行化保存之间**;**回填读(backfill)不在这个队列里 → 可与保存并发**。
- ✅ 主流程对 cursor 失效有处理:`src/scripts/tauri/chat/prompt-backfill.js:96` `isWindowedCursorInvalidError`(说明 host 设计上会在失效时 resync/重试)。
- 契约文档:`docs/CurrentState/WindowedPayload.md`(尤其 cursor 定义、以及"signature mismatch 应主要代表文件被外部修改/多进程写入,不应被应用内并发保存轻易触发"——若被应用内并发触发即是 bug)。

## 4. 为什么偏偏是"旧/长对话"

旧对话长 → 窗口化 → 需要回填 → 才会校验 cursor。短/新对话整段在内存窗口、不回填、不校验 → 不报错。所以 bug 只在**长到需要窗口化**的旧对话上出现。

## 5. 假设(分优先级)

### H1(主)❓ 小白X 插件的异步历史读 与 窗口化保存竞态
插件 fire-and-forget 异步读历史(触发窗口化回填,捕获 cursor C=size1/mtime1),与主流程保存(发消息/回复 → 文件→size2/mtime2、windowState 更新)**并发未串行** → 插件那次回填到达后端时用旧 C 校验新文件 → mismatch。
- 解释了:间歇性(竞态)、只长对话、AI 缺最近历史(窗口状态被搅乱)、"用户改过适配"(可能引入未 await/未协调的异步读)。
- **决定性测试**:**禁用小白X 插件**后重开旧对话发消息——错误消失则基本坐实。

### H2(次)❓ 保存后未刷新 cursor(上游窗口化 bug,= issue #73/#87 家族)
capture cursor 与 backfill 之间发生保存,文件变了但 backfill 仍用旧 cursor。需查:主流程"发消息→保存→组 prompt 回填"链路里,backfill 取的 cursor 是否在保存后被刷新(`windowState.cursor` 是否更新),还是用了 stale 值。

### H3(环境)❓ Android 外置存储 mtime 不稳
`/storage/emulated/` 的 FUSE/sdcardfs 层,mtime 可能被系统/同步触碰或精度不一致 → `modified_millis` 复现不出 → 即便 size 不变也 mismatch。可对同一文件多次 stat 看 mtime 是否漂移。

## 6. 代码地图(起点)

| 区域 | 文件:行 |
|---|---|
| 签名校验(报错点) | `src-tauri/.../file_chat_repository/windowed_payload_io.rs:256-267` |
| 签名计算(size+mtime) | 同上 `:54` |
| before-pages 后端读 | `.../file_chat_repository/repository_impl.rs:576` `get_chat_payload_before_lines`;group 同名在 `group_chat_repository_impl.rs:148` |
| 保存队列(只串行写) | `src/script.js:563-596` |
| 前端回填 + cursor 失效处理 | `src/scripts/tauri/chat/prompt-backfill.js`(`isWindowedCursorInvalidError:96`、`buildCursorSignature:45`) |
| 窗口状态/脏标 | `src/scripts/tauri/chat/windowed-state.js`(`markWindowedChatDirtyFromIndex`、cursor 字段) |
| 窗口化 patch/save 传输 | `src/scripts/tauri/chat/transport.js` |
| `[object Object]` toast 源(待定位) | 搜 `Context backfill failed` / 抛错处未 stringify error |
| 契约 | `docs/CurrentState/WindowedPayload.md` |

## 7. 调查步骤

1. **先做 H1 决定性测试**:禁用小白X → 复现?定方向。
2. 若 H1 成立:拿到插件适配里"读历史"那段——确认它(a)是否走 `api.chat` 的 before-pages 回填(碰 cursor)还是 `getContext().chat`(内存,安全);(b)是否在生成/CHAT_CHANGED 期间 fire-and-forget 未 await;(c)是否捕获并处理 `isWindowedCursorInvalidError`。
3. 若 H2:在主流程加日志,打印 capture cursor 与 backfill 时的 (size,mtime) 及中间是否有保存;确认保存是否刷新 `windowState.cursor`。
4. 若 H3:对该 `.jsonl` 反复 `stat` 看 mtime 是否漂移;在 Android 真机/模拟器上验。
5. 定位 `Context backfill failed [object Object]` 的抛出点(`[object Object]` = error 对象没被 stringify),改成可读错误并据此判失败来源(插件 vs 主流程)。

## 8. 候选修法(按根因)

- **H1(插件)**:适配改成 **读内存 `getContext().chat` 而非触发窗口化回填**;或读前 `await` 保存完成(host 有 `isChatSaving`/`enqueueChatSave`);或捕获 cursor 失效→重取新 cursor 重试,而非抛 toast。
- **H2(主流程)**:保存后刷新 `windowState.cursor`;backfill 取 cursor 前确认与最新文件签名一致;cursor 失效时自动 resync 重试(host 已有 `isWindowedCursorInvalidError`,补全重试路径)。**与 regenerate 窗口 bug 同类**(已有修复参考:commit `f378757` "mark windowed state dirty when regenerate trims last message")。
- **H3(环境)**:签名放宽——mtime 容差,或改用 **size + 内容/前缀 hash** 而非精确 mtime 毫秒;或对外置存储降级为 size-only 校验。

## 9. 注意 / 排除项

- ❌ **不是** Agent Memory P0 的 `stampAllMessages`(用户是原版,无此代码)。
- 小白X 插件**不在本仓库**,需用户提供适配代码才能定 H1 细节。
- 优先级:**先 H1(禁插件测试,成本最低、最可能)**,再 H2(主流程 cursor 刷新),H3 兜底。
