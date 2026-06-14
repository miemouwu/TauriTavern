# Debug 交接: 正常 ST 发送模式下 prompt 缺历史(疑似连最新一楼都没进去)

> 给在**用户真机/Mac**上调试的 agent。本机能**复现 + 抓真实发出的 payload**,这是定位的关键(我这边是只读代码、无法复现)。这是**正常 SillyTavern chat-completion 发送模式,不是 Agent 模式**——别去查 agent gateway / `agent-context-policy.js`。

## 1. 现象

- 平台 Android(`/storage/emulated/0/Android/data/...`),**普通 ST 发送**(非 agent)。
- 长对话里模型"看不到前面的消息";用户最新感觉是**连最新一楼也没进 prompt**。
- 可能伴随窗口化错误(见 §6 关联 bug:cursor signature mismatch / context backfill failed)。

## 2. 第一步(决定性):抓真实发出的 messages

**先确认 prompt 里到底有没有最新楼,再谈原因。** 普通模式下捕获最终 payload:
- 监听 `event_types.GENERATE_AFTER_DATA`(`src/script.js`):它在发请求前暴露 `generate_data`(含最终 `messages`)。打印 `generate_data.messages`,数一下有几条 chat 楼、最后一条是不是用户刚发的那楼。
- 或抓网络层实际 POST 给 provider 的 body。
- **判定**:
  - 最新楼**不在** `messages` 里 → prompt 组装阶段把它丢了(走 §5 的 H1/H3)。
  - 最新楼**在**、但更早的没有 → 是历史预算/回填上限(走 H1 token 预算 / H2 窗口化上限)。
  - `messages` 异常空/错乱 → 窗口化 `chat[]` 失同步(走 H2,关联 §6 cursor bug)。

## 3. 关键机制(✅ 已核实代码)

### 3.1 prompt 组装 + token 预算(`src/scripts/openai.js`)
- `:2378` `chatCompletion.setTokenBudget(openai_max_context, openai_max_tokens)` —— 总预算 = Context Size。
- `:1705-1759` 历史装填循环:逐条 `canAfford(chatMessage)`,放不下即 `outOfBudget=true` 停。
- **装填顺序**:先加固定块(main / 角色卡 charDescription / charPersonality / scenario / personaDescription / worldInfo / examples),**再**把 chat 历史塞进**剩余**预算(`populateChatCompletion`)。
- `:475` 设置项 `openai_max_context`;`:137` **默认 `max_4k = 4095`**;`:417` `max_context_unlocked`(解锁更大档)。

### 3.2 窗口化回填(`src/scripts/tauri/chat/prompt-backfill.js`)
- `buildGenerationChatWithBackfill`(`:175`):从最近往前回填,**三上限谁先到就停**:
  - token 预算:`Context Size × TARGET_CONTEXT_UTILIZATION`(`:15` = **0.75**)
  - 页数:`DEFAULT_MAX_PAGES`(桌面 6 / **移动 4**,`:17-18`)
  - 消息数:`DEFAULT_MAX_MESSAGES`(桌面 800 / **移动 400**,`:20-21`)
- 内存窗口 `chat[]` 只保留尾部;完整历史在磁盘,靠回填取。
- ⚠️ 注意 `:182` `if (!windowState?.cursor || !windowState?.hasMoreBefore) return { chat: sourceMessages }`——回填的**基线 `baseMessages` 必须已含最新楼**;若 `sourceMessages` 本身就缺最新楼,这里直接返回也缺。

### 3.3 窗口化 cursor 校验(后端 `src-tauri/.../file_chat_repository/windowed_payload_io.rs:256-267`)
- cursor 签名 = 文件 (size, mtime毫秒);回填/保存时按实时文件校验,不符即 `Cursor signature mismatch`。

## 4. 设置先确认(可能一步解决)
- **AI Response Configuration → Context Size(`openai_max_context`)**:默认仅 **4095(4K)**。
  - 若没调大:固定块(角色卡+世界书+系统提示+示例)很容易就吃掉接近 4K → **剩余预算 ≈ 0 → 连最新一条 chat 楼都 `canAfford` 失败、被丢**。这能解释"连最新楼都没进去"。
  - 解锁 `max_context_unlocked` + 设到模型真实窗口(deepseek 64K 等)再试。
- 顺手记录:当前 Context Size 值、角色卡+世界书+系统提示的 token 量。

## 5. 假设(分优先级)

### H1(最可能)❓ 固定块吃光预算 → 连最新楼都放不下
默认 Context Size 4K,角色卡/世界书/系统提示/示例若 ≥ ~4K,历史循环(`openai.js:1705`)第一条(最新楼)就 `canAfford` 失败 → 0 条历史。
- **Debug**:在 `:1705-1759` 循环里打印每次 `canAfford` 结果 + 当前已用/总预算;打印固定块各自 token。若进循环前剩余预算已 ≈0 → 坐实。**先验:把 Context Size 调到 32K+ 再看最新楼是否回来。**

### H2 ❓ 窗口化 `chat[]` / 回填失同步 → 生成数组缺最新楼
窗口状态错乱(或 cursor mismatch 导致回填走降级路径),`buildGenerationChatWithBackfill` 的 `baseMessages` 已缺最新楼,或回填异常。
- **Debug**:打印传入 `buildGenerationChatWithBackfill` 的 `baseMessages`(最后一条是不是最新楼)+ 返回 `chat`;看是否抛 `isWindowedCursorInvalidError`(`prompt-backfill.js:96`)走了降级。关联 §6。

### H3 ❓ prompt 组装把最新楼当成别的(continue/quiet/prefill)丢了
某些生成类型(continue / impersonate / 最后一条是 assistant 的 swipe 等)对最后一楼有特殊处理。确认复现时的生成类型(普通 send vs swipe vs continue)。
- **Debug**:在 `populateChatCompletion` 加 chat 历史处打印实际进入的消息条数与首尾。

## 6. 关联已知 bug
同分支 `docs/investigations/windowed-cursor-mismatch.md`:Android 上 cursor signature mismatch → context backfill failed → prompt 缺历史。若本问题伴随那两个报错,大概率同根(窗口化失同步)。该文档的 H1(自适配的小白X 插件异步读历史 × 窗口化保存竞态)同样可能波及本问题——**先做"禁用小白X 插件复现"测试**。

## 7. 建议调试顺序
1. **抓真实 payload**(§2)——确认最新楼到底在不在。
2. **设置先验**:Context Size 调到 32K+ 并解锁,复现是否消失(验 H1)。
3. 若设大无效:打印 §3.1 预算明细 + §3.2 `baseMessages`/返回 chat,定位丢在组装还是回填。
4. 看有没有 cursor mismatch / `isWindowedCursorInvalidError`(关联 §6),并做禁用小白X 插件测试。
5. 记录:生成类型、Context Size、固定块 token、窗口状态、是否报窗口化错误。

## 8. 排除项
- ❌ 不是 Agent 模式(不查 `agent_model_gateway` / `agent-context-policy.js` / `initialChatHistoryMessages`)。
- ❌ 不是 P0 消息身份 stamp(用户原版无此代码)。

## 关键文件
- `src/scripts/openai.js`(prompt 组装 + token 预算:`:1705-1759`、`:2378`、`:475`、`:137`、`:417`)
- `src/scripts/tauri/chat/prompt-backfill.js`(回填上限:`:175`、`:15`、`:17-21`、`:96`、`:182`)
- `src/script.js`(`GENERATE_AFTER_DATA` 捕获 payload;`populateChatCompletion` 调用点)
- `src-tauri/.../file_chat_repository/windowed_payload_io.rs:256`(cursor 签名)
- `docs/CurrentState/WindowedPayload.md`(契约)
