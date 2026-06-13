# Agent Memory & Compaction 设计 (v2)

本文件定义 Agent 的**长期记忆层(Memory)**与**上下文压缩(Compaction)**的统一设计。是实现蓝本,也是面向社区讨论的草案。

> **v2 修订**:回应首轮评审。修复水位线公式矛盾、补"消息身份"P0 前置、定义 turn/round 并拆压缩两轴、调和与 `PromptAssembly.md` 的归属冲突、世界书移出冻结前缀、verifier 成本与可选性、token 估算表述、检索粒度诚实化;新增"非线性(swipe/branch)""存储与 GC""安全"三节;并明确"前缀冻结 vs 连续裁剪"的解法。变更点标注 (Bx/Sx)。

配套阅读:`PromptAssembly.md`(前端 PromptManager 拥有真实 prompt 组装)、`Workspace.md`(persist root、checkpoint)、`RunEventJournal.md`、`ToolSystem.md`(`chat.search`/`skill.search`/`apply_patch`)。

## 0. 一句话原则

> Memory 是**原始对话(事实层)之上的派生索引**:引用事实层、可从其重建、存"带置信度的断言"。Compaction 不删事实(JSONL 仍在),只**批量**回收 prompt 空间;**绝不连续裁剪**,以保住 provider 前缀缓存。

## 1. 设计依据:为什么不抄 coding agent 的懒压缩

Coding agent 的真相在文件系统(可重读、可验证),故可懒、可有损。RP 真相只在对话——但 TauriTavern 把对话以 JSONL 持久化且 `chat.search`/`chat.read_messages` 可重读,故更接近 coding。残留差异:

| | coding 文件系统 | RP 对话 JSONL |
| --- | --- | --- |
| 状态形式 | 自描述(代码即状态) | prose,状态隐式需推断 |
| 索引手段 | grep / AST(机械) | LLM 维护的语义索引(需推断聚合) |
| 验证 | 测试 | 无,靠 provenance 回溯 |

本设计 = **coding 的空间回收骨架 + RP 的勤维护语义索引**。

## 2. 术语与两条增长轴 (B3)

- **turn(回合)** = 一条已提交的聊天消息(用户/角色)。对话历史以 turn 增长。
- **round(轮)** = 一次 run 内的工具循环迭代(`loop_runner.rs:43`)。一个 turn 内部可含数十 round。

prompt 撑大有**两条独立增长轴**,需不同压缩:

| 轴 | 增长 | 谁拥有 | 压缩手段 |
| --- | --- | --- | --- |
| **A 跨 run 历史** | 对话越来越长 | **前端 PromptManager**(组装真实 prompt,`PromptAssembly.md:7`) | 近期 N turn 逐字 + 注入 memory digest(§8.1) |
| **B run 内工具回合** | 单 run 大量 round 累积 tool turn | Rust(`model_turn.rs`) | tool-result clearing(§8.2) |

凡说"每 N 轮",一律指 **N 个 turn**,非 round。默认 N=3。

## 3. 分层与不变量

```text
事实层 = 原始对话 JSONL(唯一真相,可重读;Compaction 不从磁盘删)
   ↑ provenance: {message_id, content_sha}（见 §5）
索引层 = persist/memory/* 结构化表（派生、可重建、永不独立权威）
   ↓ 物化（仅 compaction 时进 prompt）
prompt 注入 = 薄状态头(always-on) + memory.* 主动检索 + compaction 时的 digest
```

**不变量**

- I1 事实层是唯一真相;Memory 与之冲突以事实层(及 `human_confirmed` 覆盖)为准。
- I2 `memory = derive(chat) ⊕ human_override`,任何时刻可重建。
- I3 每条 Memory 带 `source_refs`({message_id, content_sha})与时间锚。
- I4 Compaction 不丢数据(JSONL 仍在)。
- I5 已验证才写:Memory 只接收经验证闸通过的事实(verifier 漏判除外)。
- **I6 (新)** 连续裁剪被禁止:历史只由 §8.1 的批量 compaction 丢弃,以保前缀缓存(§7)。

## 4. 数据模型

`persist/memory/`(创作者可在核心表外扩展自定义表,受 §17 校验约束):

```text
persist/memory/
  entities.json    # 角色/物品/地点档案（含 relationships + address + knows/doesnt_know）
  timeline.jsonl   # append-only 时间线（挂时间锚 + 叙事框，幂等）
  threads.json     # 已埋未偿伏笔
  state.json       # 当前世界快照
  plan.json        # 前瞻 beats（planner 拥有，严格独立于 timeline）
persist/memory.meta.json     # 水位线、schema 版本、id↔position 映射缓存
persist/memory.overrides.json# human_override 层（独立断言，见 §14）
persist/digest.md            # 派生（仅 compaction 时物化进 prompt）
```

### 4.1 Entity

```json
{
  "id": "guyuan", "name": "顾远", "aliases": ["顾远"],
  "relationships": [{ "to": "zhupeiling", "type": "son_of" }],
  "address": { "to_zhupeiling": "妈/妈妈" },
  "state": { "location": "...", "knows": ["..."], "doesnt_know": ["..."] },
  "voice": "短句、克制",
  "source_refs": [{ "message_id": "m_3f2a", "content_sha": "ab12…" }],
  "last_confirmed_turn": 47, "confidence": "canon"
}
```

`relationships`+`address` 防人称/关系错;`knows`/`doesnt_know` 防全知/剧透。

### 4.2 Timeline 事件(可排序时间 + 叙事框)(S7)

```json
{
  "event_id": "hash(content_sha_of_source + canonical_summary)",
  "narration_order": 47,          // 叙述顺序（出现的 message 位置）
  "diegetic_seq": 138,            // 剧内可排序时序（单调整数，供 range 查询/“几天前”计算）
  "story_time_label": "洪荒历114年·惊蛰",  // 人类可读标签（不参与排序）
  "frame": "actual",             // actual | flashback | dream | hypothetical | rumor
  "status": "past",              // past | current | planned
  "summary": "顾远向林安安表白",
  "source_refs": [{ "message_id": "m_…", "content_sha": "…" }]
}
```

- **`diegetic_seq`**(单调整数,由 consolidation 维护)解决"`story_time` 自由文本不可排序"——`memory.timeline(range)` 与"剧内 N 天前"按它算;`story_time_label` 仅展示。
- **`frame`** 区分闪回/梦境/假设/谣言,防把闪回当当前。
- **`event_id` 基于 `content_sha` 而非位置下标**(S7/B2):消息移位不改 event_id,重建幂等。

### 4.3 置信度

`provisional`(单 turn 新断言)/ `canon`(被佐证或合并确认)/ `human_confirmed`(人工,最高,覆盖一切)。错误是"可覆盖的低置信断言",非"被删的真理"。

## 5. 消息身份(P0 前置)(B2)

TauriTavern 无稳定消息 id:`ChatMessage` 仅 `name/is_user/is_system/send_date/mes/extra`,agent 工具按 **0-based 位置**寻址,commit 用 `chat.length-1`。位置寻址对尾部操作够用,但对长期 provenance 会**烂**(删/插一楼,所有 source_refs 错位;event_id/去重/重建全崩)。

**方案(加性、不破 ST 兼容)**:

- commit 时往消息 `extra.tauritavern.msgId` 写一个稳定 uuid + `extra.tauritavern.contentSha`(消息体 sha)。ST round-trip 未知字段,导入/导出兼容。
- `source_refs = {message_id, content_sha}`。**content_sha 让"源被改"可检测**(§12 重派生的检测前提)。
- 现有流程仍按位置走;**Memory 侧维护 `message_id ↔ position` 映射**(缓存于 `memory.meta.json`,位置漂移时刷新)。
- `id` 仅需 **per-chat 唯一**(Memory 是 per persist root),不要求全局唯一——这让分叉拷贝(§13)保留 id 即可自洽。

## 6. 双 cadence(后端表更新 ≠ prompt digest 更新)(B1, 前缀冻结)

务必分开两件事的频率:

| 动作 | 频率 | 作用域 | 是否动 prompt 缓存 |
| --- | --- | --- | --- |
| **逐字尾部** | 每 turn | prompt | append,前缀保留 |
| **Memory 表合并/更新** | 每 N=3 turn | 后端 persist | **否**(只供 memory.search) |
| **digest 物化进 prompt + 丢历史(compaction)** | token ≥ 80% 窗口 | prompt | 破一次缓存 |

**水位线公式(修正 B1)**:合并处理 `(watermark, now − tail_len]`,**水位线永远落后逐字尾部 ≥ tail_len**。于是:

- 最近 `tail_len` 个 turn 永远逐字在 prompt 里 → 模型看原文,Memory 不必新鲜。
- 任一 turn 在滚出尾部之前必已合并 → 零滞后冲突。
- **swipe/regenerate 永远发生在尾部(未合并区)→ Memory 从未记录被 swipe 掉的内容 → 无需 revert**(§13.1)。
- 约束:`tail_len ≥ N` 且 `≥ 可 swipe 深度`。

注:§12 的"改源后重派生"是**允许的水位线下方再入**(纠错),与"前向合并永不重入"不矛盾——前者按 provenance 定点失效重算,后者指正常合并不回扫。

## 7. Prompt 分区与前缀冻结 (B4, 前缀冻结, B3)

```text
A 冻结前缀（字节恒定 → 前缀缓存恒命中）
   系统提示 + 工具 schema（指纹化，仅 schema 变才失效）
   + 角色卡/人设（聊天级稳定的部分）
B 记忆头/digest（仅 compaction 时更新进 prompt）
   薄状态头 + compaction 时物化的 digest
C 可压缩中段（compaction 的丢弃目标）
D 近期尾部（逐字，长度 ≥ tail_len，含活跃 tool-call/result 配对）
```

**世界书移出 A(修正 B4)**:WI 是**每 run 按需激活**(`PromptAssembly.md:64`,timed/sticky WI 必然 churn),放进"恒定 A"是错的。WI 归 B/C:要么接受 B 随 WI 变,要么对激活集做**带滞后的指纹**(激活集稳定时不重拼)。A 只留真正聊天级稳定的部分。

**前缀冻结 vs 连续裁剪(核心,见 I6)**:

- **连续裁旧消息会每轮在 A 之后断缓存**(前缀不再是上次的超集)。
- 故**禁止连续裁**;改为:窗口内纯 append(缓存友好)→ 撞 80% 做**一次** compaction(丢中段 + 物化 digest)→ 破一次 → append 恢复。
- **我们的 compaction 在 80% 抢先压制 PromptManager 的连续裁**(把上下文压回阈值下,PromptManager 的预算约 100% → 永不触发裁剪)。**PromptManager 不再连续裁是本设计对 `PromptAssembly.md` 的唯一行为要求**,需显式实现/配置。
- **digest 只在 compaction 时进 prompt**(非每 N 轮合并),故两次 compaction 之间 `[A][digest]` 字节稳定 → 大前缀命中;历史 append 前缀保留 → 近乎只有最新一条不缓存。
- DeepSeek thinking:旧轮 `reasoning_content` 只对尾部回灌,中段清除(§8.2;`encode.rs:216` 现对全部轮回灌,是现存 bug,P1 修)。

## 8. 压缩两轴 (B3)

### 8.1 跨 run 历史压缩 → 前端 prompt 组装(非 Rust phase)

不重写 Rust prompt builder(`PromptAssembly.md` 禁止)。改为前端在组装时:

```text
contextPolicy: 近期 N turn 逐字 + 其余以 memory digest 代表
```

`initialChatHistoryMessages` 从"-1 全量 / 朴素滑窗"升级为"近期 N 逐字 + digest"。**Memory 表与 digest 在后端算,digest 作为 prompt 组件由前端注入**——不碰 PromptAssembly.md 的归属。被 compaction 丢掉的老历史,正由 digest 代表。

### 8.2 run 内工具回合压缩 → Rust(tool-result clearing)

一次长 run 内:丢弃旧 tool 结果(workspace 可重读),只留近期。**必须保 tool-call/result 配对完整**(丢 assistant `tool_calls` 必连同其 tool 结果,否则 provider 400)。落点 `loop_runner.rs` / `encode.rs`。同时做旧轮 reasoning_content 清除(§7)。

## 9. 时间完整性:防穿越 / 防重放 (S7, §6.2)

- **防穿越**:事件带 `narration_order`+`diegetic_seq`+`story_time_label`+`frame`+`status`;digest 注入必带时间("(剧内3天前)…");`plan` 严格独立于 `timeline`,`planned` 绝不以 `past` 注入;`frame≠actual` 的事件不进"当前 state"。
- **防重放**:水位线 `consolidated_up_to`(§6)使合并不回扫;`event_id` 基于 `content_sha` 去重,已存即跳过。

## 10. 检索:主动调用索引层 (S1)

新增工具(与 `chat.search`/`skill.search` 同族):

```text
memory.search(query)   memory.read(id|row)   memory.timeline(range)   memory.propose(updates)
```

**诚实说明检索粒度(修正 S1)**:`summary.rs` 的 trigram 指纹是**每聊天文件一个 4096-bit Bloom 预筛**,不是行级索引。我们要**复用其技术、新写"每行指纹"**(随每次合并重建,新代码,非零成本复用)。且 Bloom 只支持**子串/词面**匹配 → **phase-1 `memory.search` 是对表的精确片段 grep,不是语义检索**;语义检索(向量)留 phase-2(预留接口)。planner 据此按 beat 拉相关行;writer 获只读 `memory.search` 落实"不确定就检索"。被动薄状态头 + 主动深检索并存。

## 11. 写入与验证闸(pre-commit) (B5, S8)

```text
writer → workspace 草稿（未进 chat、未进 memory）
   → planner 旁路审查（scratch，阅后即焚）
   → PASS: workspace.commit 进 chat；memoryDelta 入待合并队列
   → FAIL: 仅把结构化纠正注入 writer 重试（草稿覆盖，不 rewind canon）
```

- **阅后即焚对 context 成本≈0 且不破 writer 缓存**(审查是独立委派 invocation,journal 只留摘要事件)。
- **但 verifier 的真正成本是延迟(修正 B5)**:每次 commit 串行多一次非流式模型调用(草稿恒不命中缓存)+ FAIL 重试(纠正进 writer context,破其 run 内缓存)。这与"省调用治延迟"的动机相抵。故:
  - **verifier 按 profile 可选**(默认开/关待定 §20);
  - **限重试**:1 次后 commit-with-flag(`consistency_unverified`),不无限重试;
  - **可钉廉价模型**;
  - 文档须给**预期增加延迟数字**(实现期实测填入)。
- `apply_patch` 做局部修复需"同 session 先 read 再 patch"(`apply_patch.rs:103`);verifier 在独立 invocation 有自己的 session → 需先 read,额外 round(S8)。仅用于 token 级无涟漪修复;涉及 prose 逻辑打回 writer。

## 12. 纠错与冲突裁决

### 12.1 writer 文本 vs Memory:听谁(按冲突对象)

| 对象 | 例 | 谁权威 | 动作 |
| --- | --- | --- | --- |
| 不可变/结构事实 | "儿子"写成"丈夫" | Memory 赢 | 修文本 |
| 可变状态 | "在摊位"→"回家了" | writer 赢 | 更新 Memory |

`human_confirmed` 被违背 → 几乎必错;`provisional` 被违背 → writer 可能是纠正;模糊 → 升级给人。

### 12.2 逃过闸的错(事后发现)

- **检测**:`content_sha` 变化 → 源被改(§5)。
- **改源 → 按 provenance 失效并重派生**依赖它的 Memory 条目(允许的水位线下方再入,§6 注)。
- **已级联**:向前 retcon(改 confidence,认下游小矛盾);硬 retcon 一般不实用。代价随错误存活时间暴涨 → verifier 前置压小级联窗口。
- **人工修正 = 最高信号**:写入 `memory.overrides.json` 标 `human_confirmed`,survive rebuild。

## 13. 非线性:swipe 与 branch (S7)

### 13.1 swipe(叶子级)

swipe 替换最后一条内容(同槽,swipe_id),发生在**未合并的逐字尾部**(§6)→ Memory 从未记录 → **无需 revert**。约束:`tail_len ≥ 可 swipe 深度`(群聊多条尾部 AI 消息可 swipe → tail_len 取够)。

### 13.2 branch(分叉)= snapshot-fork,非 git DAG

已验证(`bookmarks.js`):`createBranch(mesId)` = `structuredClone(chat.slice(0, mesId+1))` 深拷前缀 → `saveChat` 存**新聊天文件**;子带 `main_chat`(→父)、父消息带 `extra.branches`(→子)。新聊天 = 新 stable_chat_id = **新 persist/memory root** → Memory 天然按分叉隔离。**chat 层分叉树已存在(main_chat + extra.branches)**,Memory 骑其上。

- **branch Memory(惰性,首次跑 agent 时)**:`rebuild(branch chat 0..N) ⊕ copy(parent overrides[refs ≤ N])`。因 Memory=derive(chat)、分支 chat 恰为 0..N → 重建结果 = 父在第 N turn 的 Memory,天然正确;`⊕` 父 override 前缀保住共享段人工修正。
- **不建 git DAG**:merge RP 几乎不用、传播价值低、复杂度高;snapshot-fork 对齐现成 persist-copy 模型。**内容寻址(共享前缀去重)列为存储成问题时的后续优化**。
- **id 衔接**:`structuredClone` 连 `extra` 深拷 → 分支前缀保留父的 msgId → 分支 source_refs 在分支作用域内自洽(id 仅需 per-chat 唯一,§5)。

## 14. 从原始对话重建 (S2)

```text
memory = derive(chat) ⊕ human_override_layer
rebuild = 重跑 derive(chat)，重新合并 overrides
```

- **override 表达为独立断言**(`{entity_id|event_id, field, value, scope_refs}`),存 `memory.overrides.json`,**在 derive 之后合并**——不依赖会变的 event_id 物理行。重建后按 entity/field 键重新落位。
- **derive 是 LLM 管线,非确定性** → "rebuild 与 live diff 审计"会有改写/措辞/推断噪声;审计须用**语义 diff + 阈值**(只报实质事实差异),否则不可用。
- 成本高 → 显式/分块,非常规路径。

## 15. 并发与 ACL (S5)

- **Memory 必须是独立的、对模型不可写的 root**;所有变更经 host 侧 `memory.*` 工具。workspace ACL 仅 root 级 read/write(`policy.rs`),若 memory/ 落在可写 root 下,writer 能 `workspace_write_file` 直写绕过 `memory.propose` + verifier(违反 I5)。
- `memory.*` 工具是**新的 host CAS 机制**(借鉴 `apply_patch` 的 read-before-write + 版本守卫 + 可恢复错误),**不是"复用 persistent_store 提交链路"就免费**。条目级 CAS,而非字符串 patch。
- 写权:planner 独写;writer 仅 `memory.propose`。

## 16. 存储与 GC (S4)

`persist` 现状:每 run **全量拷入拷出**所有 persistent root + 每次提交存不可变快照(`initialize_projected_roots`/`commit_persistent_state`),`timeline.jsonl` append-only **永久增长**,**删持久文件不支持**(`agent.persistent_delete_unsupported`)。移动端 IO/存储随聊天线性膨胀。须有:

- timeline 自身的**周期性压实**(旧事件聚合为 era 摘要行;通过整文件替换而非删除规避 delete 限制)。
- persist 快照的**保留策略 / GC**(老的 state 快照按 `persistStateId` 引用回收,复用现有 `prune_agent_chat_persistent_states`)。
- 评估"每 run 全量拷贝大 timeline"的成本;必要时 Memory 走差量而非全拷(需 runtime 支持,列为风险)。

## 17. 安全 (S6)

consolidation 会把对话文本(用户输入、导入聊天、被恶意卡诱导的模型输出)蒸馏进**常驻 Memory**,每 turn 重注、survive compaction/retcon、被 rebuild 忠实重建。底线:

- Memory 内容在 prompt 中**当数据处理**(分隔/转义,不可作为指令执行)。
- **digest 限大小上限**。
- 创作者自定义表须**字段类型白名单校验**(否则等于更强的世界书注入面);拒绝 instruction-like 字符串进 `state`/`voice` 等回灌字段。
- `memory.rebuild` 在不受信导入聊天上运行 = 在污染源上重建 → 视同处理不受信内容。

## 18. 落到 TauriTavern runtime (B6, B3)

| 件 | 落点 | 备注 |
| --- | --- | --- |
| **消息身份** | commit 写 `extra.tauritavern.msgId/contentSha`;Memory 侧 id↔position 映射 | **P0** |
| **token 估算** | 接现有 `TokenizerRepository::count_messages`(已存在,bundle claude/deepseek/gemma)进 gateway;**新增 model→max-context 注册表 + 计 encoded payload** | **修正 B6:基建已有,缺的是分母与编码后计数** |
| 跨 run 历史压缩 | 前端 contextPolicy:近期 N 逐字 + digest 注入 | 不重写 Rust prompt builder |
| run 内 tool clearing + reasoning 清理 | `loop_runner.rs` / `encode.rs` | 保 tool 配对 |
| 结构化 Memory + 独立不可写 root + `memory.*` CAS 工具 | `persist/`、新 host 工具 | 非免费复用 |
| 行级检索指纹 | 复用 `summary.rs` 技术、新写每行指纹 | phase-1 子串 |
| 合并 / compaction | `loop_runner.rs` 新 phase(水位线驱动);compaction 抢先压制前端连续裁 | journal `memory_consolidated`/`context_compacted` |
| 验证闸 | planner 委派回合;可选/限重试/可廉价模型 | 阅后即焚 |

## 19. 分阶段实现

- **P0** 消息身份(extra.id + contentSha + id↔position 映射)**和** token 估算接线(TokenizerRepository + max-context 注册表 + encoded 计数)。二者是一切的前置。
- **P1** DS 旧轮 reasoning 清理(现存 bug)+ 工具 schema 稳定性测试(#75.4)。
- **P2** 结构化 Memory 表 + 独立不可写 root + `memory.*` CAS 工具 + 行级 trigram 检索(单 agent)。
- **P3** 每 N turn 幂等合并(水位线 + 时间锚 + 叙事框)→ 写 Memory;80% middle-out compaction + digest 注入(前端)+ 抢先压制连续裁;`memory.rebuild`。
- **P4** pre-commit 验证闸(可选)+ 纠错状态机 + 安全/转义。
- **P5** 非线性收尾(branch 惰性重建 ⊕ override;swipe 边界测试)+ 存储 GC。
- **P6** planner/writer 分离,复用同一 Memory 层;writer `task.return` 作第一次蒸馏。

## 20. 开放决策

1. 起步形态:单 agent(内联自报)优先,还是直接 planner/writer 分离。
2. `tail_len` / N 默认值(须 `tail_len ≥ N` 且 `≥ 可 swipe 深度`)。
3. verifier 默认开还是关;重试预算;是否钉廉价模型。
4. 检索 phase-2 向量:模型、触发、与 trigram 的融合。
5. 自定义表 schema 的字段类型白名单范围。
6. timeline 压实策略与 persist 快照 GC 阈值(移动端 vs 桌面)。
7. WI 处理:接受 B 随 WI 变,还是激活集指纹 + 滞后。
