# Agent Memory & Compaction 设计

本文件定义 Agent 的**长期记忆层(Memory)**与**上下文压缩(Compaction)**的统一设计。它是后续实现必须遵守的设计蓝本,也是面向社区讨论的草案。

配套阅读:`docs/Agent/Workspace.md`(persist root)、`docs/Agent/PromptAssembly.md`(冻结快照与 context policy)、`docs/Agent/RunEventJournal.md`(append-only journal)、`docs/Agent/ToolSystem.md`(`chat.search`/`skill.search`/`workspace.apply_patch`)。

## 0. 一句话原则

> Memory 不是第二个事实源,而是**原始对话(事实层)之上的一层派生索引**。它引用事实层、可从事实层重建,存的是带置信度的"断言"而非"真理"。Compaction 不删事实,只回收 prompt 空间。

## 1. 设计依据:为什么不直接抄 coding agent 的懒压缩

Coding agent(Claude Code、Codex、CodeWhale 等)用"阈值触发的批量压缩 + 有损丢弃",因为它们的**真相在文件系统里**——忘了可以重读文件、重跑测试,丢弃安全。

RP 的真相**只在对话历史里**。但 TauriTavern 把对话以 JSONL 持久化、且可经 `chat.search` / `chat.read_messages` 重读——所以 TauriTavern 的 RP **比一般 RP 更像 coding**:对话 JSONL = coding 的文件系统。

残留差异在于:

| | coding 文件系统 | RP 对话 JSONL |
| --- | --- | --- |
| 状态形式 | 自描述(代码即状态) | prose,状态隐式(谁在哪/谁知道什么需推断) |
| 索引手段 | grep / AST(机械确定) | LLM 维护的语义索引(需推断聚合) |
| 验证 | 测试 | 无,靠 provenance 回溯 |

因此本设计 = **借 coding agent 的空间回收骨架(三区前缀、tool-result clearing、阈值)+ RP 的勤维护语义索引(结构化表、provenance、时间锚)**。

## 2. 分层与不变量

```text
事实层(Fact Layer)= 原始对话 JSONL
  - 唯一真相,可重读(chat.search / chat.read_messages)
  - Compaction 只把回合移出 prompt,绝不从磁盘删除
        ↑ provenance 引用(source_refs)
索引层(Memory Layer)= persist/memory/* 结构化表
  - 派生、可重建、永不独立权威
  - 存"带置信度与时间锚的断言"
        ↓ 物化(选择性)
Prompt 注入 = 薄状态头(always-on)+ memory.* 主动检索
```

**不变量**

- I1 事实层是唯一真相;Memory 与之冲突时,以事实层(及 `human_confirmed` 覆盖)为准。
- I2 Memory 永远可从事实层 `rebuild`(叠加 human 覆盖层)。
- I3 Memory 条目必带 `source_refs`(provenance)与时间锚。
- I4 Compaction 不造成数据丢失(JSONL 仍在磁盘)。
- I5 已验证才写:Memory 只接收经验证闸通过的事实(verifier 漏判除外)。

## 3. 数据模型

`persist/memory/` 下的结构化表(创作者可在固定核心表外扩展自定义表):

```text
persist/memory/
  entities.json    # 角色/物品/地点的规范档案
  timeline.jsonl   # append-only 时间线(挂时间锚,幂等)
  threads.json     # 已埋未偿的伏笔
  state.json       # 当前世界快照(此刻为真)
  plan.json        # 前瞻 beats(planner 拥有,严格独立于 timeline)
persist/memory.meta.json   # 水位线、schema 版本、human 覆盖层
persist/digest.md          # 派生:可选择性物化的 prompt 视图
```

### 3.1 Entity

```json
{
  "id": "guyuan",
  "name": "顾远",
  "aliases": ["顾远"],
  "relationships": [{ "to": "zhupeiling", "type": "son_of" }],
  "address": { "to_zhupeiling": "妈/妈妈" },
  "state": { "location": "...", "mood": "...", "knows": ["..."], "doesnt_know": ["..."] },
  "voice": "短句、克制",
  "source_refs": [12, 40, 47],
  "last_confirmed_turn": 47,
  "confidence": "canon"
}
```

- `relationships` + `address`(称谓)是防"人称/关系错"的命门字段(verifier 据此校验)。
- `knows` / `doesnt_know` 是防全知/防剧透的事实,RP 最易漂。

### 3.2 Timeline 事件(挂时间锚,见 §6)

```json
{
  "event_id": "ev_<hash(source_ref+canonical)>",
  "order": 47,                    // 单调真实序(message index)
  "story_time": "洪荒历114年·惊蛰", // 剧内时间(可选)
  "status": "past",              // past | current | planned
  "summary": "顾远向林安安表白",
  "source_refs": [47]
}
```

### 3.3 置信度与 provenance

每条事实存的是**断言**:

- `confidence`: `provisional`(单轮新断言,可能错)/ `canon`(被佐证或合并确认)/ `human_confirmed`(人工修正,最高,覆盖一切)。
- `source_refs`: 建立/最近佐证该事实的 message index;聚合/推断型事实为多楼/范围,标 `status:inferred`。
- `last_confirmed_turn`: 用于冲突的近期裁决。

错误因此是"低置信、可覆盖的断言",而非"被删的真理"。

## 4. 三档节奏(彻底解耦)

| 节奏 | 频率 | 职责 |
| --- | --- | --- |
| **逐字尾部** | 每轮 | 原始近期上下文,新鲜度来源 |
| **合并 → 更新 Memory** | **每 N 轮(默认 3)** | 处理水位线后回合,幂等去重 + 挂时间 + 写 Memory |
| **Compaction(回收空间)** | token ≥ 80% 窗口 | 丢弃已合并的旧回合(出 prompt,JSONL 不删) |

**关键不变量:近期逐字尾部长度 ≥ N。** 于是任何事实在滚出尾部之前必已被合并 → **零滞后冲突**。模型对最近 N 轮看原文,Memory 只对"已滚出尾部的事"权威。

为什么"每 N 轮"而非"每轮":

1. 省调用,直接缓解非流式单轮延迟(参见 `卡在 thinking` 现象)。
2. 不为"最可能被 swipe/regenerate 的最新轮"写 Memory → 避免 regen 时 revert Memory。
3. 新鲜度由逐字尾部提供,不依赖每轮写库。

合并可在"每 N 轮"基础上额外由场景切换 / 临近 compaction 机会性触发。

## 5. Prompt 分区(FrozenPrefix)

```text
A 冻结前缀(每聊天不变 → provider 前缀缓存恒命中)
   系统提示 + 工具 schema(指纹化,仅 schema 变才失效)+ 角色卡/人设/世界书
B 记忆头/digest(仅合并时更新 → 仅那时破缓存)
   薄状态头(在场实体 + 当前 state)+ 由 memory.* 主动检索补充
C 可压缩中段(Compaction GC 目标)
D 近期尾部(逐字,长度 ≥ N,含活跃 tool-call/result 配对)
```

- 缓存×压缩矛盾在此解开:A 永远命中;A+B 在一个合并窗口内稳定;C 在 compaction 时才变。
- 工具 schema 进缓存指纹(参考 issue #75.4):schema/description 漂移才主动失效缓存。
- DeepSeek thinking:旧轮 `reasoning_content` 只对尾部回灌,中段清除(对齐 CodeWhale)。

## 6. 时间完整性:防穿越 / 防重放

### 6.1 防事件穿越(过去当现在 / 计划泄漏 / 因果倒置)

- 每事件带 `order`(真实序)+ `story_time`(剧内时间)+ `status`。
- digest 注入事件时**必须带时间**("(40楼/剧内3天前)…"),让模型知道是过去而非刚发生。
- `plan.json`(未来 beats)严格独立于 `timeline`,`status:planned` 绝不以 `past` 注入。

### 6.2 防重放(同一事件双写 / 被重新叙述)

- **水位线** `consolidated_up_to`(存于 `memory.meta.json`):合并只处理 `(watermark, now]`,完成后推进水位线;**已合并回合永不重入**。
- 事件去重:`event_id = hash(source_ref + 规范化内容)`,合并时已存在即跳过。

水位线同时是 §4"每 N 轮合并"的实现支点:每 N 轮对未合并窗口跑一次幂等合并并推进水位线。

## 7. 检索:主动调用索引层

Memory 不靠"整表注入",而靠按需检索(对齐 ST 数据库脚本的结构化表 + 升级为 agentic RAG)。新增工具(与 `chat.search`/`skill.search` 同族):

```text
memory.search(query)         → 走索引返回相关 entity/thread/timeline 行
memory.read(id | table.row)  → 读具体条目
memory.timeline(range)       → 按时间/主题查
memory.propose(updates)      → writer 提议更新(不直接写)
```

- **第一阶段索引**复用 `file_chat_repository/summary.rs` 的 CJK trigram 指纹(字符三元组 + Bloom filter,无漏报有误报,做廉价预筛 + 全文确认)。仅词面/实体检索。
- **第二阶段**叠加向量索引做语义检索(预留接口)。
- planner 用 `memory.search` 按 beat 拉相关事实 → 选进 writer 的 task 契约;writer 获只读 `memory.search` 落实"不确定就检索"的防漂移。

被动 + 主动并存:薄状态头(always-on,数百 token,保定向)+ 主动深检索(省 token、更准)。

## 8. 写入与验证闸(pre-commit)

写入路径(workspace-first,verifier 前置):

```text
writer → workspace 草稿(未进 chat、未进 memory)
   → planner 旁路审查(scratch,阅后即焚)
       ├─ 冲突分类(§9)、输出 verdict + 纠正 + memoryDelta
       └─ 审查交换 discard,仅 journal 留摘要
   → PASS: workspace.commit 进 chat + 待合并的 memoryDelta 入待处理
   → FAIL: 仅把结构化纠正注入 writer 重试(草稿覆盖,不 rewind canon)
```

- **verifier 是 chat-commit 与 memory-write 的共同闸**:catch 到错 = 拒绝草稿 + 重试,**无需 rewind canon**(错文只活在 workspace 草稿,复用 checkpoint/recoverable-retry)。
- **审查阅后即焚**:审查推理不进任何持续上下文(tool-result/thinking clearing 模式),仅 journal 留紧凑记录 → per-turn 加 verifier 的持续 context 成本 ≈ 0。
- writer **只 propose** 事实;planner 验证后才写 Memory → Memory 不收未验证事实。
- 局部修复复用 `workspace.apply_patch`:verifier(LLM)决定改什么,apply_patch 精确执行(唯一匹配 + SHA CAS)。仅用于 token 级无涟漪修复;涉及 prose 逻辑仍打回 writer 重写。

## 9. 纠错与冲突裁决

### 9.1 writer 文本与 Memory 冲突,听谁

按冲突对象分类(verifier 的核心判断):

| 冲突对象 | 例 | 谁权威 | 动作 |
| --- | --- | --- | --- |
| 不可变/结构事实 | 把"儿子"写成"丈夫" | Memory 赢 | 修文本 |
| 可变状态 | "在摊位" → "回家了" | writer 赢 | 更新 Memory |

`human_confirmed` 事实被违背 → 几乎必为错;`provisional` 事实被违背 → writer 可能是纠正。模糊(LLM 判不准)→ 升级给人。

### 9.2 纠正方式

- 局部事实(称谓/人名/代词 token,无涟漪)→ verifier 经 `apply_patch` 外科修复。
- 结构性错误(整段建立在错前提)→ 打回 writer 重写。

### 9.3 逃过闸的错(事后才发现)

- 改源:编辑/regenerate 源楼 → 按 provenance 失效并**重派生**依赖它的 Memory 条目。
- 已级联(下游 build 在错事实上):向前 retcon(改 `confidence`,认下游小矛盾);硬 retcon 一般不实用。**代价随错误存活时间暴涨 → verifier 前置以压小级联窗口。**
- 人工修正 = 最高信号:写回 Memory 标 `human_confirmed`(覆盖层,survive rebuild),"一次性纠错"变"硬约束"。

## 10. 从原始对话重建

```text
memory = derive(chat) ⊕ human_override_layer
rebuild = 重跑 derive(chat),保留 human_override_layer
```

用途:污染恢复、schema 迁移(补 `address`/`knows`)、导入无 memory 的聊天、审计(rebuild 与 live diff 检测漂移)。

约束:重建**不得冲掉 `human_confirmed` 覆盖层**(人工知识不一定可从 raw chat 推导)。成本高 → 显式/分块,非常规路径。

## 11. 并发安全

Memory 编辑借鉴 `workspace/apply_patch.rs` 的范式:

- read-before-write、版本守卫(CAS)、可恢复错误。
- Memory 为结构化条目 → "带版本守卫的字段更新"(条目级 CAS),而非字符串 patch。
- 写权默认 **planner 独写**;writer 仅 `memory.propose`,避免并发写冲突。

## 12. 落到 TauriTavern runtime

| 件 | 落点 | 备注 |
| --- | --- | --- |
| token 估算(现无) | gateway encode 后 / 新 token service | 暴露"占窗口 %",**P0 前置基建**,顺带 UI 显示上下文占比 |
| 四区 prompt | `prompt_snapshot` / `prompt_assembly` | 重构现"单一冻结快照"为 A/B/C/D |
| 结构化 Memory | 扩 `persist/`,复用 `file_agent_repository/persistent_store.rs` 提交链路 | digest 派生 |
| 检索索引 + `memory.*` | 复用 `file_chat_repository/summary.rs` trigram + `skill.search` 范式 | |
| 合并 / Compaction 阶段 | `agent_runtime_service/loop_runner.rs` 新增 phase | 水位线驱动;journal `memory_consolidated` / `context_compacted` |
| 验证闸 | loop_runner / planner 委派回合 | pre-commit;阅后即焚 |
| DS thinking 清理 | `agent_model_gateway/encode.rs` | 仅尾部回灌 reasoning |

## 13. 分阶段实现

- **P0** token 估算 + 占比暴露(独立可用;缓解"卡死无反馈"焦虑)。
- **P1** DS 旧轮 reasoning 清理 + 工具 schema 稳定性测试(#75.4)。
- **P2** 四区 prompt + 结构化 Memory 表 + trigram 检索 + `memory.*`(单 agent)。
- **P3** 每 N 轮幂等合并(水位线 + 时间锚)→ 写 Memory;80% middle-out Compaction;`memory.rebuild`。
- **P4** pre-commit 验证闸 + 纠错状态机。
- **P5** planner/writer 分离,复用同一 Memory 层;writer `task.return` 作第一次蒸馏。

## 14. 开放决策

1. 起步形态:单 agent(写库内联自报)优先,还是直接 planner/writer 分离(planner 当 verifier 红利大)。
2. 近期尾部长度(turns vs tokens;须 ≥ N)。
3. 状态头大小 / digest 选择性的默认值。
4. 索引第二阶段(向量)的模型与触发。
5. 自定义表的 schema 约束与校验。
