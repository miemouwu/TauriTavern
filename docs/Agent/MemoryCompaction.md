# Agent Memory & Compaction 设计 (v3)

本文件定义 Agent 的**长期记忆层(Memory)**与**上下文压缩(Compaction)**的统一设计。是实现蓝本,也是面向社区讨论的草案。

> **v3 修订**(回应二审):拆"易变状态头(放尾部)"与"稳定 digest(在冻结前缀)"消除三时钟矛盾;变更检测改为**操作钩子(swipe/edit 事件)+ content_sha 仅作完整性校验**,区分 swipe/编辑;诚实重述跨 run 历史压缩为**扩展前端 context-policy**(非重写 Rust prompt builder)+**协同而非禁用** PromptManager 裁剪;新增 §9 **合并的原子性/失败/降级**(S3);明确 verifier 关闭时降级的不变量;存储 GC 诚实化。
> **v2 修订**(回应首轮):水位线公式、消息身份 P0、turn/round、与 PromptAssembly.md 归属、世界书出 A 区、verifier 成本、token 表述、检索粒度、非线性/存储/安全三节。变更点标注 (Bx/Sx/Nx)。

配套阅读:`PromptAssembly.md`(前端 PromptManager 拥有真实 prompt 组装)、`Workspace.md`(persist root、checkpoint)、`RunEventJournal.md`、`ToolSystem.md`。

## 0. 一句话原则

> Memory 是**原始对话(事实层)之上的派生索引**:引用事实层、可重建、存"带置信度的断言"。Compaction 不删事实(JSONL 仍在),只**批量**回收 prompt 空间,**绝不连续裁剪**以保前缀缓存。

## 1. 设计依据

Coding agent 真相在文件系统(可重读/验证),故懒压缩。RP 真相在对话——但 TauriTavern 持久化 JSONL 且可 `chat.search` 重读,更接近 coding。残差:代码自描述、可机械索引、可测;对话是 prose、状态隐式、需 LLM 推断、无测试 → 故 RP 需 LLM 维护的语义索引 + provenance 回溯。

**现状(摘要策略)**:agent runtime **零摘要/零压缩**。仅前端有 legacy `memory`(Summarize)扩展——单条扁平滚动摘要、无结构/无 provenance/有损、前端 only,是本设计的**朴素基线**;它在 legacy 聊天路径,与本设计可共存不冲突。本设计 = coding 的空间回收骨架 + RP 的结构化语义索引,替代那个 blob 摘要。

## 2. 术语与两条增长轴 (B3)

- **turn** = 一条已提交聊天消息。历史以 turn 增长。
- **round** = 一次 run 内工具循环迭代(`loop_runner.rs:43`)。一个 turn 含数十 round。

| 轴 | 增长 | 拥有者 | 压缩 |
| --- | --- | --- | --- |
| **跨 run 历史** | 对话变长 | 前端 PromptManager(`PromptAssembly.md:7`) | 近期 N turn 逐字 + digest(§8.1) |
| **run 内工具回合** | 单 run 大量 round | Rust(`model_turn.rs`) | tool-result clearing(§8.2) |

"每 N 轮"一律指 N 个 **turn**,默认 N=3。

## 3. 分层与不变量

```text
事实层 = 原始对话 JSONL（唯一真相，可重读；Compaction 不从磁盘删）
   ↑ provenance: {message_id, content_sha}
索引层 = persist/memory/*（派生、可重建、永不独立权威）
   ↓ 物化为两种 prompt 注入：
     · 稳定 digest（冻结前缀 B，仅 compaction 时更新）
     · 易变状态头（尾部 D 区，每 turn/合并可刷新，不在缓存前缀）
```

**不变量**

- I1 事实层唯一真相;冲突以事实层(及 `human_confirmed`)为准。
- I2 `memory = derive(chat) ⊕ human_override`,任何时刻可重建。
- I3 每条 Memory 带 `source_refs`({message_id, content_sha})与时间锚。
- I4 Compaction 不丢数据(JSONL 仍在)。
- I5 已验证才写(verifier 开启时);**verifier 关闭时 I5 降级**(见 §11)。
- I6 禁止连续裁剪;历史只由 §8.1 批量 compaction 丢弃。
- **I7 (N2)** 进入冻结前缀的内容(A、B-digest)在两次 compaction 之间字节稳定;一切每-turn 易变内容(状态头、最新轮)只能放尾部 D。
- **I8 (S3)** 水位线推进与"窗口 delta 完整应用"严格原子;compaction 只丢水位线之下的 turn。

## 4. 数据模型

```text
persist/memory/
  entities.json    # 角色/物品/地点（relationships + address + knows/doesnt_know）
  timeline.jsonl   # append-only 时间线（时间锚 + 叙事框，幂等）
  threads.json     # 已埋未偿伏笔
  state.json       # 当前世界快照
  plan.json        # 前瞻 beats（planner 拥有，独立于 timeline）
persist/memory.meta.json      # 水位线、schema 版本、id↔position 映射缓存
persist/memory.overrides.json # human_override 独立断言（§14）
persist/digest.md             # 派生（仅 compaction 时物化进前缀 B）
```

### 4.1 Entity

```json
{ "id":"guyuan","name":"顾远","aliases":["顾远"],
  "relationships":[{"to":"zhupeiling","type":"son_of"}],
  "address":{"to_zhupeiling":"妈/妈妈"},
  "state":{"location":"...","knows":["..."],"doesnt_know":["..."]},
  "voice":"短句、克制",
  "source_refs":[{"message_id":"m_3f2a","content_sha":"ab12…"}],
  "last_confirmed_turn":47,"confidence":"canon" }
```

### 4.2 Timeline 事件(可排序时间 + 叙事框)(S7)

```json
{ "event_id":"hash(source_content_sha + canonical_summary)",
  "narration_order":47, "diegetic_seq":138,
  "story_time_label":"洪荒历114年·惊蛰", "frame":"actual",
  "status":"past", "summary":"顾远向林安安表白",
  "source_refs":[{"message_id":"m_…","content_sha":"…"}] }
```

`diegetic_seq`(单调整数)供排序/"剧内 N 天前";`frame`∈actual/flashback/dream/hypothetical/rumor;`event_id` 基于 content_sha 而非位置 → 移位不改 id。

### 4.3 置信度

`provisional` / `canon` / `human_confirmed`(覆盖一切)。错误是可覆盖的低置信断言。

## 5. 消息身份(P0)(B2, N1)

TauriTavern 无稳定消息 id(`ChatMessage` 仅 name/is_user/is_system/send_date/mes/extra,工具按 0-based 位置寻址)。位置寻址对尾部操作够用,对长期 provenance 会烂。

- commit 时写 `extra.tauritavern.msgId`(uuid)+ `extra.tauritavern.contentSha`。ST round-trip 未知字段,兼容。
- `source_refs = {message_id, content_sha}`。`id` 仅需 **per-chat 唯一**(Memory per persist root,storage 按 `stable_chat_id→workspace_id` 键控,跨聊天重复无害)。
- **变更检测靠操作钩子,不靠 content_sha 推断(修正 N1)**:
  - **swipe / edit / delete** 是已知前端操作 → 发对应信号驱动 Memory。
  - **尾部(水位线之上、未合并)的 swipe/edit = 无 Memory 动作**(facts 尚未入库)。
  - **已合并楼(水位线之下)的 swipe 或 edit = 同义("该楼内容变了")→ 重派生该楼 provenance**(§12.2),二者动作相同、皆正确,无需区分类型字段。
  - **content_sha 仅作完整性校验**:合并/rebuild 时若发现 sha 与记录不符且无对应操作信号 → 标记"外部/意外篡改",报警而非静默。
- Memory 侧维护 `message_id ↔ position` 映射(`memory.meta.json`),位置漂移时刷新。

## 6. 三个时钟(显式分开,消除矛盾)(N2, B1)

| 动作 | 频率 | 作用域 | 缓存影响 |
| --- | --- | --- | --- |
| 逐字尾部 + **易变状态头** | 每 turn | prompt **尾部 D** | append/尾部本就每轮变,**不破前缀** |
| Memory 表合并 | 每 N=3 turn | 后端 persist | 否 |
| **稳定 digest 物化进前缀 B** + 丢历史 | token ≥ 80% | prompt **前缀 B** | 破一次 |

**关键(修正 N2)**:`memory.search` 与"易变状态头"都读**每-3-turn 的表**(同源,不打架);它们放在**尾部 D**,每 turn 刷新无所谓——尾部本就不在缓存前缀里。**冻结前缀 B 里的 digest 只在 compaction 时更新**,故两次 compaction 之间前缀字节稳定。**"always-on 状态头"≠"前缀 B"**:状态头在尾部,digest 在前缀,二者是不同位置的不同物。

**水位线(B1)**:合并处理 `(watermark, now − tail_len]`,水位线永远落后逐字尾部 ≥ tail_len。故:尾部永远逐字 → 不靠表新鲜;任一 turn 滚出尾部前必已合并 → 零滞后;swipe 永在尾部 → 无需 revert。约束 `tail_len ≥ N` 且 `≥ 可 swipe 深度`(群聊多条可 swipe 尾部 → 取够)。

§12 的"改源后重派生"是**允许的水位线下方定点再入**(纠错),与"前向合并不回扫"不矛盾。

## 7. Prompt 分区与前缀冻结 (B4, N2, 前缀冻结)

```text
A 冻结前缀（字节恒定 → 缓存恒命中）
   系统提示 + 工具 schema（指纹化） + 角色卡/人设（聊天级稳定部分）
B 稳定 digest（仅 compaction 时更新进 prompt → 两次 compaction 间稳定）
C 可压缩中段（compaction 丢弃目标）
D 尾部（逐字，≥ tail_len，含活跃 tool 配对 + 易变状态头 + memory.search 结果）
```

- **世界书出 A(B4)**:WI 每 run 按需激活(`PromptAssembly.md:64`,timed/sticky 必 churn)→ 归 B/C,或激活集**带滞后指纹**。A 只留真正稳定部分。
- **前缀冻结 vs 连续裁(I6)**:连续裁会每轮在 A 后断缓存。故窗口内纯 append → 撞 80% 做**一次** compaction → 破一次 → append 恢复。两次 compaction 间 `[A][B-digest]` 字节稳定 → 大前缀命中,历史 append 前缀保留。
- DS thinking:旧轮 `reasoning_content` 只对尾部回灌,中段清除(`encode.rs:130` 现对全部轮回灌 = 现存 bug,P1 修)。

## 8. 压缩两轴 (B3, N3)

### 8.1 跨 run 历史 → 扩展前端 context-policy(诚实重述)

**不重写 Rust prompt builder**(`PromptAssembly.md` 禁止),但**确实要扩展前端历史选择逻辑**——这是前端本职(context policy 在前端)。具体(修正 N3):

- 把 `agent-context-policy.js` 的 `applyInitialChatHistoryPolicy`(现仅 `-1` / `slice(0,N)`)扩为:**近期 N turn 逐字 + 注入 digest 组件 + 易变状态头**。Memory 表/digest 在后端算,前端仅注入,**不碰 Rust prompt builder**。
- **协同而非禁用 PromptManager 裁(关键)**:未裁候选历史已存于冻结快照(`script.js:6014` 的 `oaiMessages`),**后端可独立计 token,无循环依赖**。我们的 compaction 在 80% 触发,把 `digest+尾部` 压到**远低于** PromptManager 预算 → 其 newest-first 裁剪循环(`openai.js:1751`)**很少触发**;它仍是 failsafe,不依赖"禁用开关"(本就无此开关)。
- 工作量如实标:这比"注入个 digest"多——含前端选择逻辑 + 后端独立 token 计 + 80% 决策回路。

### 8.2 run 内工具回合 → Rust tool-result clearing

长 run 内丢旧 tool 结果(workspace 可重读),**保 tool-call/result 配对完整**(否则 provider 400)。落点 `loop_runner.rs`/`encode.rs`,同时清旧轮 reasoning。

## 9. 合并的原子性、失败与降级 (S3 — 新增)

consolidation 是**多步、多文件、可失败**操作:① 读窗口 → ② LLM 抽取 delta → ③ 写多表 → ④ 推进水位线。失败模式与契约:

- **水位线最后原子推进(I8)**:两阶段——先算出并校验**完整** delta 集,再一次性应用全部表写入,**最后**推进水位线。任一步在 ④ 前失败 → 水位线不动 → 下次重处理。
- **重处理幂等**:event_id(content_sha)去重 + entity 状态更新设为**幂等 set-to-value**(非 increment)→ 双应用无害。
- **原子多文件写**:写 staging 再原子 rename(借 `apply_patch` guarded write 范式),或先写 journal "consolidation_intent" 事件、应用后写 "consolidation_done",崩溃恢复据此幂等重做。run 级亦有 `OnRunCompleted` 全量提交兜底(中途崩则整段 delta+水位线随 run 一起不 promote)。
- **降级(合并跟不上,如 provider 持续挂)**:水位线停滞 → 不得让未合并 turn 滚出尾部。措施:**加宽 tail / 暂阻新轮**(多留逐字),或**降级用更便宜抽取**;**绝不静默丢未合并 turn**。
- **compaction 只丢水位线之下**(已合并)——明文写死,杜绝"卡住时丢未合并"导致静默丢事实。

## 10. 检索 (S1)

工具:`memory.search / read / timeline / propose`。**诚实粒度**:`summary.rs` 是**每聊天文件一个 4096-bit Bloom 预筛**,非行级索引;我们**复用技术、新写每行指纹**(随合并重建,非零成本)。Bloom 只支持**子串/词面** → **phase-1 `memory.search` 是对表精确片段 grep,非语义**;语义(向量)留 phase-2。planner 据此按 beat 拉相关行,writer 获只读 `memory.search`。

## 11. 写入与验证闸(pre-commit) (B5, N6, S8)

```text
writer → workspace 草稿 → planner 旁路审查（scratch，阅后即焚）
   PASS: workspace.commit 进 chat；memoryDelta 入待合并队列
   FAIL: 仅把结构化纠正注入 writer 重试（草稿覆盖，不 rewind canon）
```

- 阅后即焚对 context≈0 且不破 writer 缓存(审查是独立委派 invocation,journal 只留摘要)。
- **真正成本是延迟(B5)**:每次 commit 串行多一次非流式调用(草稿恒不命中缓存)+ FAIL 重试(纠正进 writer context,破其 run 内缓存)。故:
  - **verifier 按 profile 可选**;限重试(1 次后 `commit-with-flag: consistency_unverified`);可钉廉价模型;文档须给实测延迟数字。
  - `apply_patch` 局部修复需"同 session 先 read 再 patch"(`apply_patch.rs:103`),verifier 独立 invocation 要先 read,额外 round(S8)。仅 token 级无涟漪修复;涉 prose 打回 writer。
- **verifier 关闭时降级(修正 N6)**:I5 不再成立——Memory 收未验证事实;§12.2 的事后级联 retcon 成为主纠错手段(代价更高)。profile 须明示此降级。**默认开/关待定(§20)**。

## 12. 纠错与冲突裁决

### 12.1 writer vs Memory 听谁

| 对象 | 例 | 谁权威 | 动作 |
| --- | --- | --- | --- |
| 不可变/结构事实 | "儿子"写成"丈夫" | Memory | 修文本 |
| 可变状态 | "在摊位"→"回家了" | writer | 更新 Memory |

`human_confirmed` 被违背→几乎必错;`provisional`→writer 可能是纠正;模糊→升级给人。

### 12.2 逃过闸的错

- **检测靠操作信号(§5)**:swipe/edit 已合并楼 → 重派生;content_sha 不符且无操作信号 → 报篡改。
- **重派生**:按 provenance 定点失效依赖条目(允许的水位线下方再入)。
- **已级联**:向前 retcon(改 confidence);硬 retcon 一般不实用 → verifier 前置压小级联窗口。
- **人工修正**:写 `memory.overrides.json` 标 `human_confirmed`,survive rebuild。

## 13. 非线性:swipe / branch (S7)

- **swipe(叶子)**:替换最后内容,在未合并尾部 → 无需 revert。约束 `tail_len ≥ 可 swipe 深度`。
- **branch(分叉)= snapshot-fork**:已验证 `bookmarks.js:171/186` `createBranch=structuredClone(slice(0,N))→saveChat 新文件`;子带 `main_chat`、父带 `extra.branches`;新聊天=新 stable_chat_id=新 persist root。chat 层分叉树已存在,Memory 骑其上。**branch Memory 惰性**:`rebuild(branch 0..N) ⊕ copy(parent overrides[refs≤N])`。**不建 git DAG**(merge 不用、传播低值);内容寻址去重列为后续。`structuredClone` 保留 `extra.id` → 分支引用自洽。

## 14. 重建 (S2)

`memory = derive(chat) ⊕ human_override`;`rebuild = 重跑 derive(chat) 后合并 overrides`。override 表达为独立断言(`{entity_id|event_id, field, value, scope_refs}`,存 `memory.overrides.json`,derive 后按键合并,不依赖会变的物理行)。derive 非确定性 → 审计用**语义 diff + 阈值**(只报实质事实差异)。成本高 → 显式/分块。

## 15. 并发与 ACL (S5)

Memory 必须是**独立、对模型不可写的 root**;所有变更经 host `memory.*` 工具(workspace ACL 仅 root 级,`policy.rs`;memory/ 若在可写 root 下,writer 能 `workspace_write_file` 绕过)。`memory.*` 是**新 host CAS 机制**(借 `apply_patch` 的 read-before-write+版本守卫+可恢复错误),条目级 CAS,**非"复用 persistent_store 即免费"**。写权:planner 独写,writer 仅 `memory.propose`。

## 16. 存储与 GC (S4, N4)

persist 现状:每 run **全量拷入拷出**所有 root + 每次提交存**不可变快照**(`persistent_store.rs:52/191`),`timeline.jsonl` **永久增长**,**删持久文件不支持**(`:175`)。

- timeline 周期性**压实**:旧事件聚合为 era 摘要行,**整文件替换**(算 Modified,非 delete,绕开限制)。
- **诚实(N4)**:整文件替换不缩存储——每 run 仍 fork 全量不可变快照,旧未压实 timeline 留在历次快照里直到 GC。真问题是"每 run 全量拷大 timeline"。须:复用 `prune_agent_chat_persistent_states`(`agent_commands.rs:274`)按 `persistStateId` 引用回收老快照;评估必要时让 Memory 走**差量**而非全拷(需 runtime 支持,列为风险/前置)。

## 17. 安全 (S6)

consolidation 把对话文本(用户/导入/被恶意卡诱导的模型输出)蒸馏进**常驻 Memory**,每 turn 重注、survive compaction/retcon、被 rebuild 忠实重建。底线:Memory 内容在 prompt 中**当数据**(分隔/转义,不可作指令);**digest 限大小**;自定义表**字段类型白名单校验**;拒绝 instruction-like 串进 `state`/`voice`;`rebuild` 在不受信导入聊天上视同处理不受信内容。

## 18. 落到 runtime (B6, B3, N3)

| 件 | 落点 | 备注 |
| --- | --- | --- |
| 消息身份 | commit 写 `extra.tauritavern.msgId/contentSha`;id↔position 映射 | **P0** |
| token 估算 | 接现有 `TokenizerRepository::count_messages`(已存在,bundle claude/deepseek/gemma)进 gateway;**新增 model→max-context 注册表 + 计 encoded payload** | B6:缺分母与编码后计数 |
| 跨 run 历史压缩 | **扩** `agent-context-policy.js`:近期 N 逐字 + digest + 状态头;后端独立 token 计 + 80% 回路 | N3:不重写 Rust builder,但确含前端选择逻辑 |
| run 内 tool/reasoning 清理 | `loop_runner.rs`/`encode.rs` | 保 tool 配对 |
| Memory 表 + 独立不可写 root + `memory.*` CAS | `persist/`、新 host 工具 | 非免费 |
| 行级检索指纹 | 复用 `summary.rs` 技术、新写每行 | phase-1 子串 |
| 合并(原子)+ compaction | `loop_runner.rs` 新 phase;水位线驱动;§9 原子/降级 | journal intent/done/consolidated/compacted |
| 验证闸 | planner 委派;可选/限重试/可廉价 | 阅后即焚 |

## 19. 分阶段实现

- **P0** 消息身份(extra.id+contentSha+映射)**和** token 接线(TokenizerRepository + max-context 注册表 + encoded 计数)。
- **P1** DS 旧轮 reasoning 清理(现存 bug)+ 工具 schema 稳定性测试(#75.4)。
- **P2** Memory 表 + 独立不可写 root + `memory.*` CAS + 行级 trigram(单 agent)。
- **P3** 每 N turn **原子幂等合并**(水位线+时间锚+叙事框+§9 失败/降级)→ 写 Memory;80% compaction + 前端 digest 注入 + 协同 PromptManager;`memory.rebuild`。
- **P4** pre-commit 验证闸(可选)+ 纠错状态机 + 安全/转义。
- **P5** 非线性收尾(branch 惰性重建 ⊕ override;swipe 边界测试)+ 存储 GC/差量评估。
- **P6** planner/writer 分离,复用同一 Memory 层。

## 20. 开放决策

1. 起步:单 agent(内联自报)优先,还是直接 planner/writer。
2. `tail_len` / N 默认(须 `tail_len ≥ N` 且 `≥ 可 swipe 深度`)。
3. verifier 默认开/关;重试预算;是否钉廉价模型(关时 I5 降级)。
4. 检索 phase-2 向量:模型/触发/与 trigram 融合。
5. 自定义表字段白名单范围。
6. timeline 压实策略与快照 GC 阈值;是否需 Memory 差量提交(移动端)。
7. WI:接受 B 随 WI 变,还是激活集指纹+滞后。
