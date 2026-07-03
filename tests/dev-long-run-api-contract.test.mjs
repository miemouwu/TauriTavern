import test from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

async function importFresh(relativePath) {
    const modulePath = path.join(REPO_ROOT, relativePath);
    const url = `${pathToFileURL(modulePath).href}?t=${Date.now()}-${Math.random()}`;
    return import(url);
}

function createDeferred() {
    let resolve;
    let reject;
    const promise = new Promise((resolvePromise, rejectPromise) => {
        resolve = resolvePromise;
        reject = rejectPromise;
    });
    return { promise, resolve, reject };
}

function createLongRunHarness({
    deferredGeneration = false,
    startWithoutActiveChat = false,
    activeCharacter = 'bot.png',
    shujukuSnapshotSourceIndexes = null,
    shujukuApiReadyAfterReads = 0,
    windowInfoFailures = 0,
} = {}) {
    let totalCount = 0;
    const chat = [];
    const characters = [{ avatar: 'bot.png', name: 'Bot', chat: 'bot-chat' }];
    let characterId = startWithoutActiveChat ? undefined : 0;
    let chatId = startWithoutActiveChat ? undefined : 'bot-chat';
    const generateCalls = [];
    const stopCalls = [];
    const getCharactersCalls = [];
    const selectCharacterCalls = [];
    const inputEvents = [];
    const storeGetJsonCalls = [];
    const shujukuApiReads = [];
    const windowInfoCalls = [];
    const pendingGeneration = deferredGeneration ? createDeferred() : null;
    const textarea = {
        value: '',
        dispatchEvent(event) {
            inputEvents.push(event.type);
        },
    };
    const vectorField = {
        textContent: 'ready',
        getAttribute(name) {
            return name === 'data-acu-vector-index-field' ? 'status' : null;
        },
    };
    const documentMock = {
        querySelector(selector) {
            return selector === '#send_textarea' ? textarea : null;
        },
        querySelectorAll(selector) {
            return selector === '[data-acu-vector-index-field]' ? [vectorField] : [];
        },
    };
    const handle = {
        async summary() {
            return { message_count: totalCount };
        },
        metadata: {
            async get() {
                return {
                    extensions: {
                        'sp-shujuku': {
                            lastProcessedAbsFloor: totalCount - 1,
                        },
                    },
                };
            },
        },
        store: {
            async listKeys({ namespace }) {
                assert.equal(namespace, 'sp-shujuku');
                return ['snapshot__default', 'unrelated'];
            },
            async getJson({ namespace, key }) {
                assert.equal(namespace, 'sp-shujuku');
                assert.equal(key, 'snapshot__default');
                const sourceMessageIndex = Array.isArray(shujukuSnapshotSourceIndexes)
                    ? shujukuSnapshotSourceIndexes[Math.min(storeGetJsonCalls.length, shujukuSnapshotSourceIndexes.length - 1)]
                    : totalCount - 1;
                storeGetJsonCalls.push({ namespace, key, sourceMessageIndex });
                return {
                    data: {
                        memo: { sourceData: { data: [[1], [2], [3]] } },
                    },
                    sourceMessageIndex,
                };
            },
        },
        history: {
            async tail({ limit }) {
                return {
                    startIndex: Math.max(0, totalCount - limit),
                    totalCount,
                    messages: chat.slice(-limit),
                    hasMoreBefore: totalCount > limit,
                };
            },
        },
    };
    const chatApi = {
        current: {
            handle() {
                return handle;
            },
            async windowInfo() {
                windowInfoCalls.push({ generated: generateCalls.length });
                if (windowInfoCalls.length <= windowInfoFailures) {
                    throw new Error('Failed to decode chat line in chat file "bot.jsonl": incomplete utf-8 byte sequence from index 12');
                }
                return {
                    mode: 'windowed',
                    chatKind: 'character',
                    totalCount,
                    windowStartIndex: Math.max(0, totalCount - 50),
                    windowLength: Math.min(50, totalCount),
                };
            },
        },
    };
    const autoCardUpdaterApi = {
        exportTableAsJson() {
            return {
                mate: {},
                memo: { sourceData: { data: [[1], [2]] } },
                summary: { rows: [{ id: 1 }] },
            };
        },
    };
    const windowMock = {
        document: documentMock,
        __TAURITAVERN__: {
            api: {
                chat: chatApi,
            },
        },
        SillyTavern: {
            getContext() {
                return {
                    chat,
                    characters,
                    characterId,
                    chatId,
                    activeCharacter,
                    async getCharacters() {
                        getCharactersCalls.push('getCharacters');
                    },
                    async selectCharacterById(id, options) {
                        selectCharacterCalls.push({ id, options });
                        characterId = id;
                        chatId = characters[id]?.chat;
                    },
                    async generate(type, options) {
                        const prompt = textarea.value;
                        generateCalls.push({ type, options, prompt });
                        if (pendingGeneration) {
                            await pendingGeneration.promise;
                        }
                        chat.push({ is_user: true, mes: prompt });
                        chat.push({ is_user: false, mes: `reply ${generateCalls.length}` });
                        totalCount = chat.length;
                        return { ok: true };
                    },
                    stopGeneration() {
                        stopCalls.push('stop');
                    },
                };
            },
        },
    };
    Object.defineProperty(windowMock, 'AutoCardUpdaterAPI', {
        configurable: true,
        get() {
            shujukuApiReads.push({ generated: generateCalls.length });
            return shujukuApiReads.length > shujukuApiReadyAfterReads ? autoCardUpdaterApi : undefined;
        },
    });

    globalThis.window = windowMock;
    globalThis.document = documentMock;

    return {
        windowMock,
        generateCalls,
        stopCalls,
        getCharactersCalls,
        selectCharacterCalls,
        inputEvents,
        storeGetJsonCalls,
        shujukuApiReads,
        windowInfoCalls,
        pendingGeneration,
    };
}

test('dev longRun start drives SillyTavern generation for five turns and records shujuku diagnostics', async () => {
    const { generateCalls, inputEvents } = createLongRunHarness();
    const { installDevApi } = await importFresh('src/tauri/main/api/dev.js');

    installDevApi({
        async safeInvoke(command) {
            if (command === 'get_tauritavern_settings') {
                return { dev: { frontend_console_capture: false, llm_api_keep: 20 } };
            }
            throw new Error(`unexpected invoke ${command}`);
        },
    });

    const longRun = window.__TAURITAVERN__.api.dev.longRun;
    assert.equal(typeof longRun.start, 'function');
    assert.equal(typeof longRun.status, 'function');
    assert.equal(typeof longRun.cancel, 'function');
    assert.equal(typeof longRun.getLastReport, 'function');

    const report = await longRun.start({
        turns: 5,
        promptTemplate: 'ping {{turn}}/{{turns}}',
        timeoutMs: 1000,
        collectShujuku: true,
    });

    assert.equal(report.status, 'completed');
    assert.equal(report.options.turns, 5);
    assert.equal(report.turns.length, 5);
    assert.deepEqual(generateCalls.map(call => call.prompt), [
        'ping 1/5',
        'ping 2/5',
        'ping 3/5',
        'ping 4/5',
        'ping 5/5',
    ]);
    assert.deepEqual(generateCalls.map(call => call.type), ['normal', 'normal', 'normal', 'normal', 'normal']);
    assert.equal(inputEvents.filter(type => type === 'input').length, 5);
    assert.equal(report.turns.at(-1).after.windowInfo.totalCount, 10);
    assert.equal(report.turns.at(-1).messageDelta, 2);
    assert.equal(report.turns.at(-1).diagnostics.shujuku.store.namespace, 'sp-shujuku');
    assert.equal(report.turns.at(-1).diagnostics.shujuku.store.snapshotCount, 1);
    assert.equal(report.turns.at(-1).diagnostics.shujuku.table.sheetCount, 2);
    assert.equal(report.turns.at(-1).diagnostics.shujuku.vector.fields.status, 'ready');
    assert.equal(longRun.status().running, false);
    assert.equal(longRun.getLastReport().id, report.id);
});

test('dev longRun defaults to fifty turns and can run without shujuku diagnostics', async () => {
    const { generateCalls, inputEvents } = createLongRunHarness();
    const { installDevApi } = await importFresh('src/tauri/main/api/dev.js');

    installDevApi({
        async safeInvoke(command) {
            if (command === 'get_tauritavern_settings') {
                return { dev: { frontend_console_capture: false, llm_api_keep: 20 } };
            }
            throw new Error(`unexpected invoke ${command}`);
        },
    });

    const longRun = window.__TAURITAVERN__.api.dev.longRun;
    const report = await longRun.start({
        timeoutMs: 1000,
        collectShujuku: false,
    });

    assert.equal(report.status, 'completed');
    assert.equal(report.options.turns, 50);
    assert.equal(report.options.collectShujuku, false);
    assert.equal(report.turns.length, 50);
    assert.equal(generateCalls.length, 50);
    assert.equal(inputEvents.filter(type => type === 'input').length, 50);
    assert.equal(generateCalls[0].prompt, 'TauriTavern long-run stability check 1/50. Reply briefly.');
    assert.equal(generateCalls.at(-1).prompt, 'TauriTavern long-run stability check 50/50. Reply briefly.');
    assert.equal(report.turns.at(-1).after.windowInfo.totalCount, 100);
    assert.equal(report.turns.at(-1).messageDelta, 2);
    assert.equal(report.turns.every(turn => turn.messageDelta === 2), true);
    assert.equal(report.turns.at(-1).diagnostics.shujuku, undefined);
});

test('dev longRun restores the persisted active character before generation', async () => {
    const { generateCalls, getCharactersCalls, selectCharacterCalls } = createLongRunHarness({
        startWithoutActiveChat: true,
    });
    const { installDevApi } = await importFresh('src/tauri/main/api/dev.js');

    installDevApi({
        async safeInvoke(command) {
            if (command === 'get_tauritavern_settings') {
                return { dev: { frontend_console_capture: false, llm_api_keep: 20 } };
            }
            throw new Error(`unexpected invoke ${command}`);
        },
    });

    const report = await window.__TAURITAVERN__.api.dev.longRun.start({
        turns: 1,
        promptTemplate: 'restore active chat',
        timeoutMs: 1000,
        collectShujuku: false,
    });

    assert.equal(report.status, 'completed');
    assert.deepEqual(getCharactersCalls, ['getCharacters']);
    assert.deepEqual(selectCharacterCalls, [{ id: 0, options: { switchMenu: false } }]);
    assert.equal(generateCalls.length, 1);
    assert.equal(generateCalls[0].prompt, 'restore active chat');
});

test('dev longRun waits for shujuku checkpoint to catch up after generation', async () => {
    const { storeGetJsonCalls } = createLongRunHarness({
        shujukuSnapshotSourceIndexes: [-1, -1, 1],
    });
    const { installDevApi } = await importFresh('src/tauri/main/api/dev.js');

    installDevApi({
        async safeInvoke(command) {
            if (command === 'get_tauritavern_settings') {
                return { dev: { frontend_console_capture: false, llm_api_keep: 20 } };
            }
            throw new Error(`unexpected invoke ${command}`);
        },
    });

    const report = await window.__TAURITAVERN__.api.dev.longRun.start({
        turns: 1,
        promptTemplate: 'wait for shujuku',
        timeoutMs: 1000,
        collectShujuku: true,
        shujukuWaitTimeoutMs: 1000,
        shujukuWaitPollMs: 0,
    });

    assert.equal(report.status, 'completed');
    assert.equal(report.turns[0].after.windowInfo.totalCount, 2);
    assert.equal(report.turns[0].diagnostics.shujuku.store.latestSnapshot.sourceMessageIndex, 1);
    assert.equal(report.turns[0].shujukuWait.reached, true);
    assert.ok(storeGetJsonCalls.length >= 3);
});

test('dev longRun waits for shujuku runtime readiness before generation when catchup is required', async () => {
    const { generateCalls, shujukuApiReads } = createLongRunHarness({
        shujukuApiReadyAfterReads: 2,
    });
    const { installDevApi } = await importFresh('src/tauri/main/api/dev.js');

    installDevApi({
        async safeInvoke(command) {
            if (command === 'get_tauritavern_settings') {
                return { dev: { frontend_console_capture: false, llm_api_keep: 20 } };
            }
            throw new Error(`unexpected invoke ${command}`);
        },
    });

    const report = await window.__TAURITAVERN__.api.dev.longRun.start({
        turns: 1,
        promptTemplate: 'wait for shujuku runtime',
        timeoutMs: 1000,
        collectShujuku: true,
        shujukuReadyTimeoutMs: 1000,
        shujukuReadyPollMs: 0,
    });

    assert.equal(report.status, 'completed');
    assert.equal(generateCalls.length, 1);
    assert.ok(shujukuApiReads.length >= 3);
    assert.equal(shujukuApiReads.slice(0, 2).every(read => read.generated === 0), true);
    assert.equal(report.turns[0].before.diagnostics.shujuku.available, true);
});

test('dev longRun captures diagnostics when shujuku checkpoint wait times out', async () => {
    createLongRunHarness({
        shujukuSnapshotSourceIndexes: [-1],
    });
    const { installDevApi } = await importFresh('src/tauri/main/api/dev.js');

    installDevApi({
        async safeInvoke(command) {
            if (command === 'get_tauritavern_settings') {
                return { dev: { frontend_console_capture: false, llm_api_keep: 20 } };
            }
            throw new Error(`unexpected invoke ${command}`);
        },
    });

    const report = await window.__TAURITAVERN__.api.dev.longRun.start({
        turns: 1,
        promptTemplate: 'timeout diagnostics',
        timeoutMs: 1000,
        collectShujuku: true,
        shujukuWaitTimeoutMs: 1,
        shujukuWaitPollMs: 0,
    });

    assert.equal(report.status, 'failed');
    assert.match(report.turns[0].error.message, /Shujuku checkpoint did not catch up/);
    assert.equal(report.turns[0].after.windowInfo.totalCount, 2);
    assert.equal(report.turns[0].diagnostics.shujuku.store.latestSnapshot.sourceMessageIndex, -1);
});

test('dev longRun retries transient windowInfo reads during snapshots', async () => {
    const { generateCalls, windowInfoCalls } = createLongRunHarness({
        windowInfoFailures: 2,
    });
    const { installDevApi } = await importFresh('src/tauri/main/api/dev.js');

    installDevApi({
        async safeInvoke(command) {
            if (command === 'get_tauritavern_settings') {
                return { dev: { frontend_console_capture: false, llm_api_keep: 20 } };
            }
            throw new Error(`unexpected invoke ${command}`);
        },
    });

    const report = await window.__TAURITAVERN__.api.dev.longRun.start({
        turns: 1,
        promptTemplate: 'retry window info',
        timeoutMs: 1000,
        collectShujuku: true,
        shujukuWaitTimeoutMs: 1000,
        shujukuWaitPollMs: 0,
    });

    assert.equal(report.status, 'completed');
    assert.equal(generateCalls.length, 1);
    assert.ok(windowInfoCalls.length >= 4);
    assert.equal(report.turns[0].messageDelta, 2);
    assert.equal(report.turns[0].before.windowInfo.totalCount, 0);
    assert.equal(report.turns[0].after.windowInfo.totalCount, 2);
});

test('dev longRun falls back to backend settings when context active character is blank', async () => {
    const { generateCalls, getCharactersCalls, selectCharacterCalls } = createLongRunHarness({
        startWithoutActiveChat: true,
        activeCharacter: '',
    });
    const { installDevApi } = await importFresh('src/tauri/main/api/dev.js');
    const invokeCalls = [];

    installDevApi({
        async safeInvoke(command) {
            invokeCalls.push(command);
            if (command === 'get_tauritavern_settings') {
                return { dev: { frontend_console_capture: false, llm_api_keep: 20 } };
            }
            if (command === 'get_sillytavern_settings') {
                return {
                    settings: JSON.stringify({
                        active_character: 'bot.png',
                        active_group: null,
                    }),
                };
            }
            throw new Error(`unexpected invoke ${command}`);
        },
    });

    const report = await window.__TAURITAVERN__.api.dev.longRun.start({
        turns: 1,
        promptTemplate: 'restore from backend settings',
        timeoutMs: 1000,
        collectShujuku: false,
    });

    assert.equal(report.status, 'completed');
    assert.deepEqual(getCharactersCalls, ['getCharacters']);
    assert.deepEqual(selectCharacterCalls, [{ id: 0, options: { switchMenu: false } }]);
    assert.equal(generateCalls.length, 1);
    assert.equal(generateCalls[0].prompt, 'restore from backend settings');
    assert.equal(invokeCalls.includes('get_sillytavern_settings'), true);
});

test('dev longRun restore failures include active character diagnostics', async () => {
    createLongRunHarness({
        startWithoutActiveChat: true,
        activeCharacter: 'missing.png',
    });
    const { installDevApi } = await importFresh('src/tauri/main/api/dev.js');

    installDevApi({
        async safeInvoke(command) {
            if (command === 'get_tauritavern_settings') {
                return { dev: { frontend_console_capture: false, llm_api_keep: 20 } };
            }
            throw new Error(`unexpected invoke ${command}`);
        },
    });

    await assert.rejects(
        () => window.__TAURITAVERN__.api.dev.longRun.start({
            turns: 1,
            timeoutMs: 1000,
            collectShujuku: false,
        }),
        /Unable to restore active character chat before dev\.longRun.*activeCharacter="missing\.png".*characterCount=1.*matchingIndex=-1/s,
    );
});

test('dev longRun restore diagnostics include backend settings failures', async () => {
    createLongRunHarness({
        startWithoutActiveChat: true,
        activeCharacter: '',
    });
    const { installDevApi } = await importFresh('src/tauri/main/api/dev.js');

    installDevApi({
        async safeInvoke(command) {
            if (command === 'get_tauritavern_settings') {
                return { dev: { frontend_console_capture: false, llm_api_keep: 20 } };
            }
            if (command === 'get_sillytavern_settings') {
                throw new Error('settings command unavailable');
            }
            throw new Error(`unexpected invoke ${command}`);
        },
    });

    await assert.rejects(
        () => window.__TAURITAVERN__.api.dev.longRun.start({
            turns: 1,
            timeoutMs: 1000,
            collectShujuku: false,
        }),
        /persistedSelectionError="settings command unavailable"/,
    );
});

test('dev longRun cancel stops generation and returns a cancelled report', async () => {
    const { pendingGeneration, stopCalls } = createLongRunHarness({ deferredGeneration: true });
    const { installDevApi } = await importFresh('src/tauri/main/api/dev.js');

    installDevApi({
        async safeInvoke(command) {
            if (command === 'get_tauritavern_settings') {
                return { dev: { frontend_console_capture: false, llm_api_keep: 20 } };
            }
            throw new Error(`unexpected invoke ${command}`);
        },
    });

    const longRun = window.__TAURITAVERN__.api.dev.longRun;
    const reportPromise = longRun.start({
        turns: 2,
        promptTemplate: 'cancel {{turn}}',
        timeoutMs: 1000,
    });

    await Promise.resolve();
    assert.equal(longRun.status().running, true);
    assert.equal(longRun.status().currentTurn, 1);

    const cancelResult = await longRun.cancel('test requested');
    assert.deepEqual(cancelResult, { cancelled: true, reason: 'test requested' });
    assert.equal(stopCalls.length, 1);

    pendingGeneration.resolve();
    const report = await reportPromise;

    assert.equal(report.status, 'cancelled');
    assert.equal(report.turns.length, 1);
    assert.equal(report.cancelReason, 'test requested');
    assert.equal(longRun.status().running, false);
});
