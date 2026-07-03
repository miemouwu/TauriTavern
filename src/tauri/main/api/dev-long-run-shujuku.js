// @ts-check

import { errorToPlain } from './dev-long-run-diagnostics.js';

export const DEFAULT_SHUJUKU_READY_TIMEOUT_MS = 60000;
export const DEFAULT_SHUJUKU_READY_POLL_MS = 250;
export const DEFAULT_SHUJUKU_WAIT_TIMEOUT_MS = 120000;
export const DEFAULT_SHUJUKU_WAIT_POLL_MS = 1000;

/**
 * @param {number} ms
 */
function delay(ms) {
    if (!ms) {
        return Promise.resolve();
    }

    return new Promise((resolve) => setTimeout(resolve, ms));
}

function nowMs() {
    return Date.now();
}

/**
 * @param {any} hostWindow
 */
function isShujukuRuntimeReady(hostWindow) {
    const api = hostWindow?.AutoCardUpdaterAPI;
    return Boolean(
        hostWindow?.__ACU_STAR_DB_III_LOADED__
        || (api && typeof api.exportTableAsJson === 'function'),
    );
}

/**
 * @param {any} hostWindow
 * @param {{
 *   timeoutMs: number;
 *   pollMs: number;
 * }} options
 */
export async function waitForShujukuRuntimeReady(hostWindow, options) {
    const startedAt = nowMs();
    let attempts = 0;

    while (nowMs() - startedAt <= options.timeoutMs) {
        attempts += 1;
        if (isShujukuRuntimeReady(hostWindow)) {
            return {
                ready: true,
                attempts,
                elapsedMs: nowMs() - startedAt,
            };
        }

        if (nowMs() - startedAt >= options.timeoutMs) {
            break;
        }
        await delay(options.pollMs);
    }

    const error = new Error(`Shujuku runtime did not become ready within ${options.timeoutMs}ms`);
    error.details = {
        ready: false,
        attempts,
        elapsedMs: nowMs() - startedAt,
        hasAutoCardUpdaterAPI: Boolean(hostWindow?.AutoCardUpdaterAPI),
        acuLoadedFlag: Boolean(hostWindow?.__ACU_STAR_DB_III_LOADED__),
    };
    throw error;
}

/**
 * @param {any} hostWindow
 * @param {string} namespace
 */
async function readLatestShujukuCheckpoint(hostWindow, namespace) {
    const handle = hostWindow?.__TAURITAVERN__?.api?.chat?.current?.handle?.();
    if (!handle?.store || typeof handle.store.listKeys !== 'function' || typeof handle.store.getJson !== 'function') {
        throw new Error('TauriTavern chat store API is unavailable');
    }

    const keys = await handle.store.listKeys({ namespace });
    const snapshotKeys = Array.isArray(keys)
        ? keys.filter((key) => String(key).startsWith('snapshot__'))
        : [];
    if (snapshotKeys.length === 0) {
        throw new Error(`No shujuku snapshot keys found in namespace ${namespace}`);
    }

    const key = snapshotKeys[snapshotKeys.length - 1];
    const snapshot = await handle.store.getJson({ namespace, key });
    const sourceMessageIndex = Number(snapshot?.sourceMessageIndex);

    return {
        key,
        sourceMessageIndex: Number.isFinite(sourceMessageIndex) ? sourceMessageIndex : null,
    };
}

/**
 * @param {any} hostWindow
 * @param {{
 *   namespace: string;
 *   targetMessageIndex: number;
 *   timeoutMs: number;
 *   pollMs: number;
 * }} options
 */
export async function waitForShujukuCheckpoint(hostWindow, options) {
    const startedAt = nowMs();
    let attempts = 0;
    let latest = null;
    let lastError = null;

    while (nowMs() - startedAt <= options.timeoutMs) {
        attempts += 1;
        try {
            latest = await readLatestShujukuCheckpoint(hostWindow, options.namespace);
            if (latest.sourceMessageIndex !== null && latest.sourceMessageIndex >= options.targetMessageIndex) {
                return {
                    reached: true,
                    targetMessageIndex: options.targetMessageIndex,
                    attempts,
                    elapsedMs: nowMs() - startedAt,
                    latest,
                };
            }
        } catch (error) {
            lastError = errorToPlain(error);
        }

        if (nowMs() - startedAt >= options.timeoutMs) {
            break;
        }
        await delay(options.pollMs);
    }

    const latestText = latest?.sourceMessageIndex ?? 'unavailable';
    const errorText = lastError ? `; last error: ${lastError.message}` : '';
    const error = new Error(
        `Shujuku checkpoint did not catch up to message ${options.targetMessageIndex} within ${options.timeoutMs}ms; latest=${latestText}${errorText}`,
    );
    error.details = {
        reached: false,
        targetMessageIndex: options.targetMessageIndex,
        attempts,
        elapsedMs: nowMs() - startedAt,
        latest,
        lastError,
    };
    throw error;
}
