// @ts-check

import { ensureActiveChatReady } from './dev-long-run-active-chat.js';
import { captureLongRunSnapshot, errorToPlain } from './dev-long-run-diagnostics.js';
import {
    DEFAULT_SHUJUKU_READY_POLL_MS,
    DEFAULT_SHUJUKU_READY_TIMEOUT_MS,
    DEFAULT_SHUJUKU_WAIT_POLL_MS,
    DEFAULT_SHUJUKU_WAIT_TIMEOUT_MS,
    waitForShujukuCheckpoint,
    waitForShujukuRuntimeReady,
} from './dev-long-run-shujuku.js';

const DEFAULT_TURNS = 50;
const DEFAULT_TIMEOUT_MS = 120000;
const DEFAULT_PROMPT_TEMPLATE = 'TauriTavern long-run stability check {{turn}}/{{turns}}. Reply briefly.';
const DEFAULT_GENERATION_TYPE = 'normal';
const DEFAULT_SHUJUKU_NAMESPACE = 'sp-shujuku';
const DEFAULT_TAIL_LIMIT = 3;

/**
 * @param {unknown} value
 * @param {string} label
 * @param {number} fallback
 */
function optionalPositiveInteger(value, label, fallback) {
    if (value === undefined || value === null || value === '') {
        return fallback;
    }

    const number = typeof value === 'number' ? value : Number(value);
    if (!Number.isSafeInteger(number) || number <= 0) {
        throw new Error(`${label} must be a positive integer`);
    }
    return number;
}

/**
 * @param {unknown} value
 * @param {string} label
 * @param {number} fallback
 */
function optionalNonNegativeInteger(value, label, fallback) {
    if (value === undefined || value === null || value === '') {
        return fallback;
    }

    const number = typeof value === 'number' ? value : Number(value);
    if (!Number.isSafeInteger(number) || number < 0) {
        throw new Error(`${label} must be a non-negative integer`);
    }
    return number;
}

/**
 * @param {unknown} value
 * @param {string} fallback
 */
function optionalString(value, fallback) {
    if (value === undefined || value === null) {
        return fallback;
    }
    const text = String(value);
    return text || fallback;
}

/**
 * @param {number} ms
 */
function delay(ms) {
    if (!ms) {
        return Promise.resolve();
    }

    return new Promise((resolve) => setTimeout(resolve, ms));
}

/**
 * @param {any} snapshot
 */
function getSnapshotTotalCount(snapshot) {
    return Number(
        snapshot?.windowInfo?.totalCount
        ?? snapshot?.tail?.totalCount
        ?? snapshot?.chatLength
        ?? 0,
    );
}

/**
 * @param {string} template
 * @param {{ turn: number; turns: number; runId: string; startedAt: string }} input
 */
function renderPromptTemplate(template, input) {
    return template
        .replaceAll('{{turn}}', String(input.turn))
        .replaceAll('{{turns}}', String(input.turns))
        .replaceAll('{{runId}}', input.runId)
        .replaceAll('{{startedAt}}', input.startedAt);
}

/**
 * @param {any} hostWindow
 */
function requireChatInput(hostWindow) {
    const documentRef = hostWindow?.document ?? globalThis.document;
    const textarea = documentRef?.querySelector?.('#send_textarea');
    if (!textarea) {
        throw new Error('Chat input #send_textarea is unavailable');
    }
    return textarea;
}

/**
 * @param {any} target
 * @param {any} hostWindow
 * @param {string} type
 */
function dispatchDomEvent(target, hostWindow, type) {
    if (typeof target?.dispatchEvent !== 'function') {
        return;
    }

    const EventCtor = hostWindow?.Event ?? globalThis.Event;
    let event = { type, bubbles: true };
    if (typeof EventCtor === 'function') {
        try {
            event = new EventCtor(type, { bubbles: true });
        } catch {
            event = { type, bubbles: true };
        }
    }
    target.dispatchEvent(event);
}

/**
 * @param {any} hostWindow
 * @param {string} prompt
 */
function setChatInput(hostWindow, prompt) {
    const textarea = requireChatInput(hostWindow);
    textarea.value = prompt;
    dispatchDomEvent(textarea, hostWindow, 'input');
    dispatchDomEvent(textarea, hostWindow, 'change');
}

/**
 * @param {Promise<any>} promise
 * @param {number} timeoutMs
 * @param {string} label
 * @param {() => void | Promise<void>} onTimeout
 */
async function withTimeout(promise, timeoutMs, label, onTimeout) {
    let timeoutId;
    const timeoutPromise = new Promise((_, reject) => {
        timeoutId = setTimeout(async () => {
            try {
                await onTimeout();
            } catch {
                // The original timeout error is more useful to callers.
            }
            reject(new Error(`${label} timed out after ${timeoutMs}ms`));
        }, timeoutMs);
    });

    try {
        return await Promise.race([promise, timeoutPromise]);
    } finally {
        clearTimeout(timeoutId);
    }
}

/**
 * @param {unknown} input
 */
function normalizeOptions(input = {}) {
    const options = input && typeof input === 'object' ? /** @type {any} */ (input) : {};
    const turns = optionalPositiveInteger(options.turns, 'turns', DEFAULT_TURNS);
    const timeoutMs = optionalPositiveInteger(options.timeoutMs, 'timeoutMs', DEFAULT_TIMEOUT_MS);
    const settleMs = optionalNonNegativeInteger(options.settleMs, 'settleMs', 0);
    const tailLimit = optionalPositiveInteger(options.tailLimit, 'tailLimit', DEFAULT_TAIL_LIMIT);
    const generationOptions = options.generationOptions && typeof options.generationOptions === 'object'
        ? { ...options.generationOptions }
        : {};

    return {
        turns,
        timeoutMs,
        settleMs,
        tailLimit,
        promptTemplate: optionalString(options.promptTemplate ?? options.userPromptTemplate, DEFAULT_PROMPT_TEMPLATE),
        generationType: optionalString(options.generationType, DEFAULT_GENERATION_TYPE),
        generationOptions,
        collectShujuku: options.collectShujuku !== false,
        shujukuNamespace: optionalString(options.shujukuNamespace, DEFAULT_SHUJUKU_NAMESPACE),
        verifyShujukuCatchup: options.collectShujuku !== false && options.verifyShujukuCatchup !== false,
        shujukuReadyTimeoutMs: optionalPositiveInteger(options.shujukuReadyTimeoutMs, 'shujukuReadyTimeoutMs', DEFAULT_SHUJUKU_READY_TIMEOUT_MS),
        shujukuReadyPollMs: optionalNonNegativeInteger(options.shujukuReadyPollMs, 'shujukuReadyPollMs', DEFAULT_SHUJUKU_READY_POLL_MS),
        shujukuWaitTimeoutMs: optionalPositiveInteger(options.shujukuWaitTimeoutMs, 'shujukuWaitTimeoutMs', DEFAULT_SHUJUKU_WAIT_TIMEOUT_MS),
        shujukuWaitPollMs: optionalNonNegativeInteger(options.shujukuWaitPollMs, 'shujukuWaitPollMs', DEFAULT_SHUJUKU_WAIT_POLL_MS),
        stopOnError: options.stopOnError !== false,
        verifyMessageGrowth: options.verifyMessageGrowth !== false,
    };
}

/**
 * @param {{
 *   getHostWindow?: () => any;
 *   idFactory?: () => string;
 * }} [deps]
 */
export function createDevLongRunApi(deps = {}) {
    const getHostWindow = deps.getHostWindow ?? (() => window);
    const idFactory = deps.idFactory ?? (() => `tt-longrun-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`);
    const safeInvoke = deps.safeInvoke;
    let activeRun = null;
    let lastReport = null;

    async function stopActiveGeneration(reason) {
        const hostWindow = getHostWindow();
        try {
            const context = hostWindow?.SillyTavern?.getContext?.();
            if (typeof context?.stopGeneration === 'function') {
                context.stopGeneration();
            }
        } catch {
            // Cancellation remains best-effort; the report records the cancel reason.
        }
        if (activeRun) {
            activeRun.cancelRequested = true;
            activeRun.cancelReason = reason;
            activeRun.abortController.abort();
        }
    }

    return {
        status() {
            if (!activeRun) {
                return {
                    running: false,
                    lastReportSummary: lastReport ? {
                        id: lastReport.id,
                        status: lastReport.status,
                        turns: lastReport.turns.length,
                        startedAt: lastReport.startedAt,
                        finishedAt: lastReport.finishedAt,
                    } : null,
                };
            }

            return {
                running: true,
                id: activeRun.id,
                currentTurn: activeRun.currentTurn,
                requestedTurns: activeRun.options.turns,
                startedAt: activeRun.startedAt,
                cancelRequested: activeRun.cancelRequested,
            };
        },
        getLastReport() {
            return lastReport;
        },
        async cancel(reason = 'cancelled') {
            if (!activeRun) {
                return { cancelled: false, reason: String(reason || 'cancelled') };
            }

            const cancelReason = String(reason || 'cancelled');
            await stopActiveGeneration(cancelReason);
            return { cancelled: true, reason: cancelReason };
        },
        async start(input = {}) {
            if (activeRun) {
                throw new Error('dev.longRun is already running');
            }

            const hostWindow = getHostWindow();
            const context = await ensureActiveChatReady(hostWindow, safeInvoke);
            const options = normalizeOptions(input);
            const id = idFactory();
            const startedAt = new Date().toISOString();
            const run = {
                id,
                startedAt,
                currentTurn: 0,
                options,
                cancelRequested: false,
                cancelReason: '',
                abortController: new AbortController(),
            };
            const report = {
                id,
                status: 'running',
                startedAt,
                finishedAt: null,
                options: {
                    turns: options.turns,
                    timeoutMs: options.timeoutMs,
                    settleMs: options.settleMs,
                    generationType: options.generationType,
                    collectShujuku: options.collectShujuku,
                    shujukuNamespace: options.shujukuNamespace,
                    verifyShujukuCatchup: options.verifyShujukuCatchup,
                    shujukuReadyTimeoutMs: options.shujukuReadyTimeoutMs,
                    shujukuReadyPollMs: options.shujukuReadyPollMs,
                    shujukuWaitTimeoutMs: options.shujukuWaitTimeoutMs,
                    shujukuWaitPollMs: options.shujukuWaitPollMs,
                    stopOnError: options.stopOnError,
                    verifyMessageGrowth: options.verifyMessageGrowth,
                },
                turns: [],
                errors: [],
            };

            activeRun = run;

            try {
                for (let turn = 1; turn <= options.turns; turn += 1) {
                    if (run.cancelRequested) {
                        report.status = 'cancelled';
                        report.cancelReason = run.cancelReason;
                        break;
                    }

                    run.currentTurn = turn;
                    const turnStartedAt = new Date().toISOString();
                    const prompt = renderPromptTemplate(options.promptTemplate, {
                        turn,
                        turns: options.turns,
                        runId: id,
                        startedAt,
                    });
                    let before = null;
                    let after = null;
                    let messageDelta = 0;
                    let shujukuReady = null;

                    try {
                        if (options.verifyShujukuCatchup) {
                            shujukuReady = await waitForShujukuRuntimeReady(hostWindow, {
                                timeoutMs: options.shujukuReadyTimeoutMs,
                                pollMs: options.shujukuReadyPollMs,
                            });
                        }

                        before = await captureLongRunSnapshot(hostWindow, options);
                        setChatInput(hostWindow, prompt);
                        const generationOptions = {
                            ...options.generationOptions,
                            signal: options.generationOptions.signal ?? run.abortController.signal,
                        };
                        const generationResult = await withTimeout(
                            Promise.resolve(context.generate(options.generationType, generationOptions)),
                            options.timeoutMs,
                            `Generation turn ${turn}`,
                            () => stopActiveGeneration(`timeout at turn ${turn}`),
                        );
                        await delay(options.settleMs);
                        after = await captureLongRunSnapshot(hostWindow, options);
                        const beforeCount = getSnapshotTotalCount(before);
                        const afterCount = getSnapshotTotalCount(after);
                        messageDelta = afterCount - beforeCount;

                        if (options.verifyMessageGrowth && messageDelta <= 0) {
                            throw new Error(`Generation turn ${turn} did not increase the message count`);
                        }

                        let shujukuWait = null;
                        if (options.verifyShujukuCatchup) {
                            shujukuWait = await waitForShujukuCheckpoint(hostWindow, {
                                namespace: options.shujukuNamespace,
                                targetMessageIndex: afterCount - 1,
                                timeoutMs: options.shujukuWaitTimeoutMs,
                                pollMs: options.shujukuWaitPollMs,
                            });
                            after = await captureLongRunSnapshot(hostWindow, options);
                        }

                        report.turns.push({
                            turn,
                            prompt,
                            startedAt: turnStartedAt,
                            finishedAt: new Date().toISOString(),
                            messageDelta,
                            generationResult,
                            shujukuReady,
                            shujukuWait,
                            before,
                            after,
                            diagnostics: after.diagnostics,
                        });
                    } catch (error) {
                        const plainError = errorToPlain(error);
                        if (!after) {
                            try {
                                after = await captureLongRunSnapshot(hostWindow, options);
                                if (before) {
                                    const beforeCount = getSnapshotTotalCount(before);
                                    const afterCount = getSnapshotTotalCount(after);
                                    messageDelta = afterCount - beforeCount;
                                }
                            } catch (snapshotError) {
                                after = {
                                    at: new Date().toISOString(),
                                    error: errorToPlain(snapshotError),
                                    diagnostics: {},
                                };
                            }
                        }
                        report.errors.push({
                            turn,
                            error: plainError,
                        });
                        report.turns.push({
                            turn,
                            prompt,
                            startedAt: turnStartedAt,
                            finishedAt: new Date().toISOString(),
                            messageDelta,
                            shujukuReady,
                            before,
                            after,
                            diagnostics: after?.diagnostics ?? {},
                            error: plainError,
                        });

                        if (run.cancelRequested) {
                            report.status = 'cancelled';
                            report.cancelReason = run.cancelReason;
                            break;
                        }

                        if (options.stopOnError) {
                            report.status = 'failed';
                            break;
                        }
                    }

                    if (run.cancelRequested) {
                        report.status = 'cancelled';
                        report.cancelReason = run.cancelReason;
                        break;
                    }
                }

                if (report.status === 'running') {
                    report.status = report.errors.length > 0 ? 'completed_with_errors' : 'completed';
                }

                return report;
            } finally {
                report.finishedAt = new Date().toISOString();
                lastReport = report;
                activeRun = null;
            }
        },
    };
}
