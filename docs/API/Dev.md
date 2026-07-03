# `window.__TAURITAVERN__.api.dev` — API 参考

TauriTavern 为开发者工具、调试面板与扩展作者提供的规范化调试 API。

> 设计目标：让调用方依赖稳定宿主 ABI，而不是直接依赖 Tauri 事件名、Rust 命令名或某个 Settings 面板的内部实现。

## 0. 快速上手

```js
await (window.__TAURITAVERN__?.ready ?? window.__TAURITAVERN_MAIN_READY__);
const dev = window.__TAURITAVERN__.api.dev;
```

## 1. `frontendLogs`

```js
const entries = await dev.frontendLogs.list({ limit: 50 });

const unsubscribe = await dev.frontendLogs.subscribe((entry) => {
  console.log('[frontend]', entry.level, entry.message);
});
```

### 方法

| 方法 | 返回值 | 说明 |
| --- | --- | --- |
| `list(options?)` | `Promise<FrontendLogEntry[]>` | 获取当前已捕获的前端日志尾部 |
| `subscribe(handler)` | `Promise<unsubscribe>` | 订阅新增前端日志 |
| `getConsoleCaptureEnabled()` | `Promise<boolean>` | 读取 console capture 开关 |
| `setConsoleCaptureEnabled(enabled)` | `Promise<void>` | 设置 console capture 开关 |

### `FrontendLogEntry`

```ts
type FrontendLogEntry = {
  id: number;
  timestampMs: number;
  level: 'debug' | 'info' | 'warn' | 'error';
  message: string;
  target?: string;
};
```

### 语义

- 前端日志 capture 开关由宿主统一管理。
- 调用方不应再自行读写相关 `localStorage` key。
- `unsubscribe` 可安全重复调用。
- 为避免 DEBUG/INFO 噪声挤掉关键告警，宿主按级别独立保留：`debug` 400 条、`info` 300 条、`warn`+`error` 共 100 条。
- `message` 是用于 UI 展示的 **preview**：可能被截断；对象参数会被摘要化。需要完整请求/响应体请使用 `llmApiLogs.getRaw()` 或导出日志。

## 2. `backendLogs`

```js
const recent = await dev.backendLogs.tail({ limit: 100 });

const unsubscribe = await dev.backendLogs.subscribe((entry) => {
  console.log('[backend]', entry.target, entry.message);
});
```

### 方法

| 方法 | 返回值 | 说明 |
| --- | --- | --- |
| `tail(options?)` | `Promise<BackendLogEntry[]>` | 获取当前后端日志尾部 |
| `subscribe(handler)` | `Promise<unsubscribe>` | 订阅新增后端日志 |

### `BackendLogEntry`

```ts
type BackendLogEntry = {
  id: number;
  timestampMs: number;
  level: 'DEBUG' | 'INFO' | 'WARN' | 'ERROR';
  target: string;
  message: string;
};
```

### 语义

- 宿主负责共享后端日志流。
- 多个订阅者并存时，底层流的启停由宿主统一引用计数，不应互相踩踏。
- `message` 可能被截断以保证性能；完整排查请以文件日志/导出 bundle 为准。

## 3. `llmApiLogs`

```js
const index = await dev.llmApiLogs.index({ limit: 20 });
const preview = await dev.llmApiLogs.getPreview(index[0].id);
const raw = await dev.llmApiLogs.getRaw(index[0].id);
```

### 方法

| 方法 | 返回值 | 说明 |
| --- | --- | --- |
| `index(options?)` | `Promise<LlmApiLogIndexEntry[]>` | 获取最近几条请求索引 |
| `getPreview(id)` | `Promise<LlmApiLogPreview>` | 获取适合 UI 展示的预览 |
| `getRaw(id)` | `Promise<LlmApiLogRaw>` | 获取完整原始请求/响应 |
| `subscribeIndex(handler)` | `Promise<unsubscribe>` | 订阅新增索引项 |
| `getKeep()` | `Promise<number>` | 读取保留条数设置 |
| `setKeep(value)` | `Promise<void>` | 设置保留条数 |

### `LlmApiLogIndexEntry`

```ts
type LlmApiLogIndexEntry = {
  id: number;
  timestampMs: number;
  level: 'INFO' | 'ERROR';
  ok: boolean;
  source: string;
  model: string | null;
  endpoint: string;
  durationMs: number;
  stream: boolean;
};
```

### `LlmApiLogPreview`

```ts
type LlmApiLogPreview = {
  id: number;
  timestampMs: number;
  level: 'INFO' | 'ERROR';
  ok: boolean;
  source: string;
  model: string | null;
  endpoint: string;
  durationMs: number;
  stream: boolean;
  errorMessage: string | null;
  requestReadable: string;
  responseReadable: string;
  responseRawKind: 'json' | 'sse' | null;
};
```

### `LlmApiLogRaw`

```ts
type LlmApiLogRaw = {
  id: number;
  requestRaw: string;
  responseRaw: string;
  responseRawKind: 'json' | 'sse' | null;
};
```

### 语义

- `getPreview()` / `getRaw()` 会从磁盘读取对应的 log 文件（按需加载，不常驻内存）。
- `requestReadable/responseReadable` 为开发者调试默认不截断（内容过大时可能影响 UI 打开速度）；完整内容也可用 `getRaw()` 或导出 bundle 获取。
- `requestReadable/responseReadable` 会显示 provider 返回的可见/摘要化 reasoning，例如 `[reasoning]`、`[thinking]`、`[thought]` 块；signature、`thoughtSignature`、`encrypted_content` 等 provider-private continuation 只显示 `native_state=present` 标记，不当作可解释文本展开。

## 4. `longRun`

`longRun` 用于在真实 TauriTavern 窗口内自动驱动多轮生成，验证前端生成入口、消息落盘、windowed chat、扩展 hook 与开发诊断链路。

```js
const report = await dev.longRun.start({
  turns: 5,
  promptTemplate: 'long-run smoke {{turn}}/{{turns}}',
  timeoutMs: 120000,
  collectShujuku: true,
});

console.log(report.status, report.turns.at(-1)?.diagnostics);
```

### 方法

| 方法 | 返回值 | 说明 |
| --- | --- | --- |
| `start(options?)` | `Promise<LongRunReport>` | 启动一轮窗口内生成压测；同一时间只允许一个 run |
| `cancel(reason?)` | `Promise<{ cancelled: boolean; reason: string }>` | 请求取消当前 run，并 best-effort 调用 `SillyTavern.getContext().stopGeneration()` |
| `status()` | `LongRunStatus` | 获取当前运行状态或最近一次报告摘要 |
| `getLastReport()` | `LongRunReport \| null` | 读取最近一次完整报告 |

### `LongRunOptions`

```ts
type LongRunOptions = {
  turns?: number;              // 默认 50
  promptTemplate?: string;     // 支持 {{turn}} / {{turns}} / {{runId}} / {{startedAt}}
  timeoutMs?: number;          // 单轮生成超时，默认 120000
  settleMs?: number;           // 每轮生成后额外等待，便于扩展 debounce 后采样
  generationType?: string;     // 默认 normal
  generationOptions?: object;  // 透传给 SillyTavern context.generate()
  collectShujuku?: boolean;    // 默认 true
  shujukuNamespace?: string;   // 默认 sp-shujuku
  stopOnError?: boolean;       // 默认 true
  verifyMessageGrowth?: boolean; // 默认 true
};
```

### 报告语义

- `longRun.start()` 会写入 `#send_textarea`，然后调用真实的 `SillyTavern.getContext().generate()`，不会绕过前端生成链路。
- 每轮都会采样 `api.chat.current.windowInfo()`、聊天尾部摘要、消息数增量与诊断信息。
- `collectShujuku` 开启时，会额外采样：
  - `AutoCardUpdaterAPI.exportTableAsJson()` 的表级摘要；
  - 当前 chat handle 的 `sp-shujuku` store keys / snapshot count / latest snapshot 摘要；
  - 当前 chat metadata extension 里的 `sp-shujuku` progress；
  - 页面上 `[data-acu-vector-index-field]` 暴露的向量索引状态字段。
- 报告只存摘要，不存完整 prompt 历史、完整表格或完整向量，以降低日志体积与敏感数据扩散风险。

## 5. `mobile`

`mobile` 用于在 Android/iOS 真机上采集当前 WebView 与移动端布局运行时快照。它只记录诊断元数据，不记录 URL query/hash、输入框内容、聊天正文或 prompt。

```js
const snapshot = await dev.mobile.logSnapshot({ reason: 'manual' });
console.log(snapshot.platform.android, snapshot.layout.safeInsets);
```

### 方法

| 方法 | 返回值 | 说明 |
| --- | --- | --- |
| `snapshot(options?)` | `MobileDebugSnapshot` | 返回当前移动端运行快照，不写日志 |
| `logSnapshot(options?)` | `Promise<MobileDebugSnapshot>` | 返回当前移动端运行快照，并写入前端日志 ring buffer 与后端日志转发流 |

### `MobileDebugSnapshot`

```ts
type MobileDebugSnapshot = {
  version: number;
  timestampMs: number;
  reason: string;
  platform: {
    userAgent: string;
    platform: string;
    language: string;
    maxTouchPoints: number;
    android: boolean;
    ios: boolean;
  };
  viewport: {
    innerWidth: number;
    innerHeight: number;
    devicePixelRatio: number;
    visualViewport: null | {
      width: number;
      height: number;
      offsetLeft: number;
      offsetTop: number;
      scale: number;
    };
    screen: null | {
      width: number;
      height: number;
      availWidth: number;
      availHeight: number;
    };
  };
  layout: {
    safeInsets: { top: number; right: number; bottom: number; left: number };
  };
  runtime: {
    mobileRuntimeCompat: boolean;
    overlayCompat: boolean;
    embeddedRuntimeProfile: string | null;
    mobileSurfaceCount: number;
  };
  document: {
    readyState: string;
    visibilityState: string;
    activeElement: null | {
      tagName: string;
      id: string;
      className: string;
      mobileSurface: string;
      imeSurface: string;
    };
    location: { origin: string; pathname: string };
  };
  host: { abiVersion: number };
};
```

### 入口

内置 `TauriTavern Version` 系统扩展提供 **Mobile Debug Snapshot** 按钮，会调用 `dev.mobile.logSnapshot({ reason: 'manual' })`，弹出当前快照，并把同一份摘要写入 debug bundle 可导出的前端/后端日志。

## 6. `exportBundle()`

```js
const zipPath = await dev.exportBundle();
```

### 方法

| 方法 | 返回值 | 说明 |
| --- | --- | --- |
| `exportBundle()` | `Promise<string>` | 导出 debug bundle（zip）并返回保存路径 |

### 语义

- 导出内容包含后端文件日志、前端日志 snapshot、LLM API Logs（含 raw）与设置/版本信息。
- 导出文件可能包含 prompt/响应体等敏感信息；分享前请自行检查。

## 7. 边界与稳定性

- `tauritavern-backend-log`
- `tauritavern-llm-api-log`
- `devlog_*`

以上事件名和命令名都属于宿主内部实现细节，不是第三方扩展 Public Contract。扩展应只依赖 `api.dev`。
