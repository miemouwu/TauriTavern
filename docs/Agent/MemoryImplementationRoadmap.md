# Agent Memory & Compaction 实现路线图(P0–P3)

> 配套《MemoryCompaction.md》(v3 设计蓝本)。本文件是**实现路线图**:把 v3 的 P0–P3(核心记忆环)拆成可执行、可测的工作单元,固化跨切面决策,并记录与 v3 的**有意分歧**(Memory 用 SQLite)。P4–P6、向量检索、存储 GC/差量不在本轮。

## 0. 范围

**本轮(P0–P3 = 核心记忆环):**

- **P0** 消息身份 + token 估算(先做,完整 TDD)
- **P1** DeepSeek 旧轮 `reasoning_content` 清理(现存 bug)+ 工具 schema 稳定性测试
- **P2** 结构化 Memory 表 + 独立不可写 root + `memory.*` CAS + 行级检索
- **P3** 每 N turn 原子幂等合并 + 80% compaction + 前端 digest 注入 + `memory.rebuild`

**显式不做(本轮外):** P4 验证闸/安全白名单 · P5 非线性收尾/存储 GC/差量 · P6 planner/writer 分离 · phase-2 向量检索。

## 1. 跨切面决策

| # | 决策 | 选择 | 备注 |
|---|---|---|---|
| 1 | 起步形态(§20.1) | **单 agent 内联自报** | 合并为 runtime phase;planner/writer 分离留 P6 |
| 2 | 默认值(§20.2) | **N=3**、**tail_len=6**(≥N、≥可 swipe 深度;群聊可调高);profile 可配 | |
| 3 | token 注册表 | `model→max_context` 覆盖 claude/deepseek/gemma/gemini/openai 家族 + 保守 fallback;80% 触发 compaction | |
| 4 | **Memory 存储** | **SQLite(authoritative 派生层)**,事实层仍 JSONL | 见 §1.1 |

### 1.1 与 v3 的有意分歧:Memory 用 SQLite,替代「文件 + 手写 trigram」

v3 §10 复用 `summary.rs` 的 per-file Bloom + 新写每行 trigram 指纹做子串检索。本实现改用 **SQLite**:

- **检索**:FTS5 `tokenize='trigram'` —— 与 v3 同思路(trigram 子串、CJK 友好),但**内建、增量维护、带索引**;phase-2 向量留 `sqlite-vec` 接口。
- **`timeline(range)`**:`diegetic_seq` B-tree 范围查询,取代 grep JSONL。
- **§9 原子合并**:多表 delta 写 + 推进水位线 = **单事务**(ACID),取代「staging + rename」。
- **§15 条目级 CAS**:`UPDATE … WHERE version = ?`,取代手写 read-before-write。
- **依赖**:`rusqlite`(`bundled` + FTS5);桌面 / Android / iOS 全支持(移动端 SQLite 原生,反而理想)。
- **事实层不变**:chat 仍 JSONL → 仍满足 I2「memory = derive(chat),可重建」(rebuild = drop tables + 重派生)。
- **持久化集成(P2 定稿,见 §7)**:memory.db 在 run commit 边界内 WAL-checkpoint + 整文件快照 → run rollback 原子回退 Memory;rebuild 为兜底。
- **透明度代价**:`.db` 非 git-diff → 加 `memory.export`(dump 成 JSON)缓解。

## 2. P0 — 消息身份 + token 估算(先做)

### 2.1 消息身份

- **数据**:`MessageExtra`(`domain/models/chat.rs`)加 typed `tauritavern: Option<TauritavernMeta> { msgId: String /*uuid v4*/, contentSha: String }`。`MessageExtra` 已有 `#[serde(flatten)] additional` → ST 未知字段 round-trip 已保 → **纯加性改动,低风险**。
- **stamp 落点**:chat 持久化处(`AgentToolEffect::ChatCommitRequested` 处理 + chat-save 路径,非 `commit.rs`——它只发 effect)。单一 helper `stamp_identity(&mut ChatMessage)`:无 `msgId` 补 uuid,(重)算 `contentSha`。覆盖 user + assistant。
- **`contentSha`** = `sha256(mes)`(活动正文;swipe/edit 改 `mes` → sha 变 → §5 完整性校验触发)。
- **`id↔position`**:P0 用 scan resolver(从消息 `extra` 重建,无新存储);持久缓存随 P2 进 SQLite `message_index` 表。→ **P0 不依赖 SQLite**。
- **已定决策**:① 在中心持久化点 stamp(所有消息);② `contentSha = sha256(mes)`。

### 2.2 token 估算

- **注册表**:新 `model → max_context`(claude/deepseek/gemma/gemini/openai 家族 + 保守 fallback)。
- **挂点**:`agent_model_gateway` 在 `encode.rs` 产出**编码后 provider payload** 之后,调现有 `TokenizerRepository::count_messages(model, &payload)`(`domain/repositories/tokenizer_repository.rs`)→ `PromptBudget { tokens, max_context, ratio }`。补 v3 B6 缺的「分母 + 编码后计数」。
- **产出**:`ratio` 供 P3 的 80% compaction 触发;P0 仅计算 / 暴露 / journal。

### 2.3 P0 测试(TDD)

stamp 幂等;未知字段 round-trip 保留;`contentSha` 稳定 / 改 `mes` 即变;resolver 在插入/删除位移下正确;注册表查找 + fallback;编码后 vs 原始 token 计数;`ratio` 计算。落 `agent_model_gateway/tests.rs` + chat 模型测试。

## 3. P1 — reasoning 清理 + 工具 schema 稳定性

- `encode.rs:130` 现对**全部轮**回灌 DeepSeek `reasoning_content` = 现存 bug → 改为**仅尾部回灌、中段清除**(§7)。保 tool-call/result 配对完整。
- 工具 schema 稳定性测试(指纹化:schema 不变则缓存不失效;对应 #75.4)。
- 测试:多轮编码后中段无 `reasoning_content`、尾部有;schema 指纹幂等。

## 4. P2 — Memory 表(SQLite)+ `memory.*` CAS + 检索

- **新 persist root** `persist/memory/`(独立、模型不可写;ACL `policy.rs` root 级 → 不置于可写 root 下,杜绝 `workspace_write_file` 绕过 §15)。
- **SQLite schema**(`memory.db`):
  - `entities(id, name, aliases, relationships, address, state, voice, last_confirmed_turn, confidence, source_refs, version)`
  - `timeline(event_id PK, narration_order, diegetic_seq, story_time_label, frame, status, summary, source_refs)` + index(diegetic_seq)
  - `threads`、`state`、`plan`
  - `overrides(target, field, value, scope_refs)`(human_override 独立断言,§14)
  - `message_index(msg_id, position, content_sha)`(P0 resolver 的持久化)
  - `meta(watermark, schema_version)`
  - FTS5 虚表 `entities_fts` / `timeline_fts`(`tokenize='trigram'`)
- **`memory.*` host 工具(CAS)**:`memory.search / read / timeline / propose`。`propose` = 条目级 CAS(`UPDATE … WHERE version = ?`),借 `apply_patch` read-before-write 范式。本轮单 agent → runtime 合并写;`propose` 供 agent 入队提议。
- **检索**:phase-1 FTS5 trigram 子串(planner 按 beat 拉行;writer 只读 `search`)。
- 测试:schema 迁移;CRUD + CAS 版本冲突;FTS5 CJK 子串召回;`timeline` range 查询;ACL 拒绝直写。

## 5. P3 — 原子合并 + compaction + digest 注入

- **合并(每 N=3 turn)**:runtime 新 phase(`loop_runner.rs`),水位线驱动,处理 `(watermark, now − tail_len]`。① 读窗口 → ② LLM 抽 delta → ③ **单事务**写多表 → ④ 事务内推进 `watermark`(I8 原子)。`event_id`(content_sha)去重 + 幂等 set-to-value → 重处理无害。
- **失败 / 降级(§9)**:任一步在 ④ 前失败 → 水位线不动 → 重处理;provider 持续挂 → 加宽 tail / 暂阻新轮,**绝不丢未合并 turn**;compaction **只丢水位线之下**。journal:`consolidation_intent/done`、`consolidated/compacted`。
- **compaction(token ≥ 80%)**:一次性丢中段 + 物化 digest 进冻结前缀 B(§7);两次 compaction 间 `[A][B-digest]` 字节稳定。
- **前端 digest 注入(§8.1)**:扩 `agent-context-policy.js` 的 `applyInitialChatHistoryPolicy`:近期 N 逐字 + digest + 易变状态头;后端独立算 token(读 `script.js:6014` 冻结快照 `oaiMessages`)+ 80% 回路;**协同**(非禁用)PromptManager newest-first 裁(`openai.js:1751`,仍作 failsafe)。
- **`memory.rebuild`**:drop tables + 重派生 ⊕ overrides;审计用语义 diff + 阈值。
- 测试:水位线推进原子性(④ 前崩 → 不推进);重处理幂等;compaction 只丢已合并;digest 注入后前缀字节稳定;前端 context-policy 单测。

## 6. 测试策略

- **Rust**:per-phase 单测随码落 `*/tests.rs`;`pnpm run check`(guardrails + tsc + contract)每改动先跑;`cargo test`(src-tauri)。
- **前端**:contract tests(`tests/**/*.test.mjs`)覆盖 context-policy 扩展。
- **集成**:小型真实 chat 跑 合并 → compaction → rebuild,断言 Memory 一致 + 前缀字节稳定。

## 7. 风险与待定

- **memory.db × per-run 快照/rollback**(P2 定稿):快照整 `.db`(WAL-checkpoint)vs 置于 run 边界外靠自身事务 + journal;倾向前者(原子 rollback),`rebuild` 兜底。
- **`max_context` 具体数值**:各 model family 待填官方/实测值;fallback 取保守(如 8k)。
- **tail_len 群聊**:多条尾部可 swipe → 群聊按可 swipe 深度调高。
- **每 run 全量拷 `memory.db` 成本**(§16/N4):大库时评估差量;本轮先全拷,GC/差量留 P5。
- **WI 处理(§20.7)**:P3 compaction 前缀 —— 接受 B 随 WI 变 or 激活集指纹 + 滞后;P3 内定。
