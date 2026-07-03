// @ts-check

/**
 * @param {unknown} value
 */
export function errorToPlain(value) {
    if (value instanceof Error) {
        const plain = {
            name: value.name,
            message: value.message,
            stack: value.stack,
        };
        const details = /** @type {any} */ (value).details;
        if (details !== undefined) {
            try {
                plain.details = JSON.parse(JSON.stringify(details));
            } catch {
                plain.details = String(details);
            }
        }
        return plain;
    }

    return {
        name: 'Error',
        message: String(value),
    };
}

const TRANSIENT_HISTORY_READ_RETRY_LIMIT = 5;
const TRANSIENT_HISTORY_READ_RETRY_DELAY_MS = 100;
const TRANSIENT_HISTORY_READ_PATTERNS = [
    /cursor signature mismatch/i,
    /early eof/i,
    /incomplete utf-?8/i,
    /failed to decode chat line/i,
    /failed to get chat summary/i,
];

/**
 * @param {unknown} error
 */
function isTransientHistoryReadError(error) {
    const message = error instanceof Error ? error.message : String(error ?? '');
    return TRANSIENT_HISTORY_READ_PATTERNS.some((pattern) => pattern.test(message));
}

/**
 * @param {number} ms
 */
function delay(ms) {
    return new Promise((resolve) => setTimeout(resolve, ms));
}

/**
 * @template T
 * @param {() => Promise<T>} operation
 */
async function withTransientHistoryReadRetry(operation) {
    let lastError = null;
    for (let attempt = 0; attempt <= TRANSIENT_HISTORY_READ_RETRY_LIMIT; attempt += 1) {
        try {
            return await operation();
        } catch (error) {
            lastError = error;
            if (!isTransientHistoryReadError(error) || attempt >= TRANSIENT_HISTORY_READ_RETRY_LIMIT) {
                throw error;
            }
            await delay(TRANSIENT_HISTORY_READ_RETRY_DELAY_MS);
        }
    }
    throw lastError;
}

/**
 * @param {unknown} message
 */
function summarizeMessage(message) {
    if (!message || typeof message !== 'object') {
        return null;
    }

    const entry = /** @type {any} */ (message);
    const text = String(entry.mes ?? entry.text ?? entry.content ?? '');
    return {
        name: entry.name,
        isUser: Boolean(entry.is_user),
        isSystem: Boolean(entry.is_system),
        textLength: text.length,
        textPreview: text.slice(0, 160),
    };
}

/**
 * @param {unknown} value
 */
function countRows(value) {
    const candidate = /** @type {any} */ (value);
    if (Array.isArray(candidate?.sourceData?.data)) {
        return candidate.sourceData.data.length;
    }
    if (Array.isArray(candidate?.data)) {
        return candidate.data.length;
    }
    if (Array.isArray(candidate?.rows)) {
        return candidate.rows.length;
    }
    return null;
}

/**
 * @param {unknown} value
 */
function countColumns(value) {
    const candidate = /** @type {any} */ (value);
    if (Array.isArray(candidate?.sourceData?.headers)) {
        return candidate.sourceData.headers.length;
    }
    if (Array.isArray(candidate?.headers)) {
        return candidate.headers.length;
    }
    if (Array.isArray(candidate?.columns)) {
        return candidate.columns.length;
    }
    if (Array.isArray(candidate?.rows) && candidate.rows[0] && typeof candidate.rows[0] === 'object') {
        return Object.keys(candidate.rows[0]).length;
    }
    return null;
}

/**
 * @param {unknown} tableData
 */
function summarizeTableData(tableData) {
    if (!tableData || typeof tableData !== 'object') {
        return {
            available: false,
            sheetCount: 0,
            sheets: {},
        };
    }

    const sheets = {};
    const keys = Object.keys(tableData).filter((key) => key !== 'mate');
    for (const key of keys.slice(0, 50)) {
        const sheet = /** @type {any} */ (tableData)[key];
        if (!sheet || typeof sheet !== 'object') {
            continue;
        }

        sheets[key] = {
            rows: countRows(sheet),
            columns: countColumns(sheet),
            name: typeof sheet.name === 'string' ? sheet.name : undefined,
        };
    }

    return {
        available: true,
        sheetCount: keys.length,
        sheets,
        truncated: keys.length > 50,
    };
}

/**
 * @param {any} hostWindow
 */
function collectVectorDomFields(hostWindow) {
    const documentRef = hostWindow?.document ?? globalThis.document;
    const fields = {};
    const nodes = documentRef?.querySelectorAll?.('[data-acu-vector-index-field]') ?? [];
    for (const node of Array.from(nodes)) {
        const key = node?.getAttribute?.('data-acu-vector-index-field');
        if (!key) {
            continue;
        }
        fields[String(key)] = String(node?.textContent ?? '').trim();
    }
    return fields;
}

/**
 * @param {any} hostWindow
 */
async function collectFrontendLogTail(hostWindow) {
    const listLogs = hostWindow?.__TAURITAVERN__?.api?.dev?.frontendLogs?.list;
    if (typeof listLogs !== 'function') {
        return {
            available: false,
            entries: [],
        };
    }

    try {
        const entries = await listLogs({ limit: 80 });
        const relevant = Array.isArray(entries)
            ? entries.filter((entry) => {
                const text = String(entry?.message ?? '');
                return /shujuku|AutoCard|ACU|插件|extension|Could not activate|failed to load|error/i.test(text);
            }).slice(-30)
            : [];

        return {
            available: true,
            count: Array.isArray(entries) ? entries.length : 0,
            entries: relevant.map((entry) => ({
                id: entry?.id,
                level: entry?.level,
                target: entry?.target,
                message: entry?.message,
            })),
        };
    } catch (error) {
        return {
            available: false,
            entries: [],
            error: errorToPlain(error),
        };
    }
}

/**
 * @param {any} hostWindow
 */
async function collectExtensionRuntimeDiagnostics(hostWindow) {
    const documentRef = hostWindow?.document ?? globalThis.document;
    const scripts = Array.from(documentRef?.scripts ?? [])
        .filter((script) => /SP-Shujuku|LittleWhiteBox|third-party/i.test(`${script.id} ${script.src}`))
        .map((script) => ({
            id: script.id,
            type: script.type,
            src: script.src,
            async: Boolean(script.async),
            loaded: script.dataset?.tauritavernLoaded,
        }));

    const styles = Array.from(documentRef?.querySelectorAll?.('link[rel="stylesheet"]') ?? [])
        .filter((link) => /SP-Shujuku|LittleWhiteBox|third-party/i.test(`${link.id} ${link.href}`))
        .map((link) => ({
            id: link.id,
            href: link.href,
            loaded: link.dataset?.tauritavernLoaded,
        }));

    const api = hostWindow?.AutoCardUpdaterAPI;
    return {
        globals: {
            hasTauriTavern: Boolean(hostWindow?.__TAURITAVERN__),
            hasSillyTavernGetContext: typeof hostWindow?.SillyTavern?.getContext === 'function',
            hasTavernHelper: Boolean(hostWindow?.TavernHelper),
            hasAutoCardUpdaterAPI: Boolean(api),
            autoCardUpdaterAPIKeys: api && typeof api === 'object' ? Object.keys(api).sort().slice(0, 50) : [],
            acuLoadedFlag: Boolean(hostWindow?.__ACU_STAR_DB_III_LOADED__),
        },
        scripts,
        styles,
        frontendLogs: await collectFrontendLogTail(hostWindow),
    };
}

/**
 * @param {any} hostWindow
 * @param {{ shujukuNamespace: string }} options
 */
async function collectShujukuDiagnostics(hostWindow, options) {
    const namespace = options.shujukuNamespace;
    const api = hostWindow?.AutoCardUpdaterAPI;
    const result = {
        available: Boolean(api),
        table: null,
        store: {
            namespace,
            keys: [],
            keyCount: 0,
            snapshotCount: 0,
            snapshotKeys: [],
        },
        progress: null,
        vector: {
            fields: collectVectorDomFields(hostWindow),
        },
        errors: [],
    };

    if (api && typeof api.exportTableAsJson === 'function') {
        try {
            result.table = summarizeTableData(await api.exportTableAsJson());
        } catch (error) {
            result.errors.push({ source: 'AutoCardUpdaterAPI.exportTableAsJson', error: errorToPlain(error) });
        }
    }

    const handle = hostWindow?.__TAURITAVERN__?.api?.chat?.current?.handle?.();
    if (!handle) {
        return result;
    }

    try {
        const metadata = await handle.metadata?.get?.();
        result.progress = metadata?.extensions?.[namespace] ?? null;
    } catch (error) {
        result.errors.push({ source: 'chat.metadata.get', error: errorToPlain(error) });
    }

    try {
        const keys = await handle.store?.listKeys?.({ namespace });
        if (Array.isArray(keys)) {
            const snapshotKeys = keys.filter((key) => String(key).startsWith('snapshot__'));
            result.store.keys = keys;
            result.store.keyCount = keys.length;
            result.store.snapshotKeys = snapshotKeys;
            result.store.snapshotCount = snapshotKeys.length;

            if (snapshotKeys.length > 0 && typeof handle.store?.getJson === 'function') {
                const latestKey = snapshotKeys[snapshotKeys.length - 1];
                const snapshot = await handle.store.getJson({ namespace, key: latestKey });
                result.store.latestSnapshot = {
                    key: latestKey,
                    sourceMessageIndex: snapshot?.sourceMessageIndex,
                    table: summarizeTableData(snapshot?.data),
                };
            }
        }
    } catch (error) {
        result.errors.push({ source: 'chat.store', error: errorToPlain(error) });
    }

    return result;
}

/**
 * @param {any} hostWindow
 * @param {{
 *   collectShujuku: boolean;
 *   shujukuNamespace: string;
 *   tailLimit: number;
 * }} options
 */
export async function captureLongRunSnapshot(hostWindow, options) {
    const context = hostWindow?.SillyTavern?.getContext?.();
    const chat = Array.isArray(context?.chat) ? context.chat : [];
    const chatApi = hostWindow?.__TAURITAVERN__?.api?.chat;
    const handle = chatApi?.current?.handle?.();
    const snapshot = {
        at: new Date().toISOString(),
        chatLength: chat.length,
        lastMessage: summarizeMessage(chat.at?.(-1) ?? chat[chat.length - 1]),
        windowInfo: null,
        tail: null,
        diagnostics: {},
    };

    if (typeof chatApi?.current?.windowInfo === 'function') {
        try {
            snapshot.windowInfo = await withTransientHistoryReadRetry(() => chatApi.current.windowInfo());
        } catch (error) {
            snapshot.windowInfo = {
                error: errorToPlain(error),
            };
        }
    }

    if (handle && typeof handle.history?.tail === 'function') {
        try {
            const tail = await withTransientHistoryReadRetry(() => handle.history.tail({ limit: options.tailLimit }));
            snapshot.tail = {
                startIndex: tail?.startIndex,
                totalCount: tail?.totalCount,
                hasMoreBefore: Boolean(tail?.hasMoreBefore),
                messages: Array.isArray(tail?.messages) ? tail.messages.map(summarizeMessage) : [],
            };
        } catch (error) {
            snapshot.tail = {
                error: errorToPlain(error),
            };
        }
    }

    if (options.collectShujuku) {
        snapshot.diagnostics.extensionRuntime = await collectExtensionRuntimeDiagnostics(hostWindow);
        snapshot.diagnostics.shujuku = await collectShujukuDiagnostics(hostWindow, {
            shujukuNamespace: options.shujukuNamespace,
        });
    }

    return snapshot;
}
