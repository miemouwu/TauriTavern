// @ts-check

const MOBILE_DEBUG_SNAPSHOT_VERSION = 1;
const MOBILE_DEBUG_LOG_TARGET = 'mobile-debug';
const MOBILE_DEBUG_EXTENSION_LOG_VERSION = 1;
const SECRET_KEY_RE = /(?:api[_-]?key|access[_-]?token|refresh[_-]?token|authorization|password|secret|chatid|chat_id|sessionid|session_id)/i;
const SECRET_VALUE_RE = /\b(?:sk|meow|cat)-?[A-Za-z0-9_-]{8,}\b/g;

const SAFE_INSET_VARS = /** @type {const} */ ({
    top: '--tt-inset-top',
    right: '--tt-inset-right',
    bottom: '--tt-inset-bottom',
    left: '--tt-inset-left',
});

/**
 * @param {unknown} value
 * @param {number} fallback
 */
function finiteNumber(value, fallback = 0) {
    const number = Number(value);
    return Number.isFinite(number) ? number : fallback;
}

/**
 * @param {string} value
 * @param {number} maxLength
 */
function trimPreview(value, maxLength = 160) {
    const text = String(value || '').trim();
    if (text.length <= maxLength) {
        return text;
    }
    return `${text.slice(0, Math.max(0, maxLength - 1))}...`;
}

/**
 * @param {unknown} value
 * @param {string} fallback
 * @param {number} maxLength
 */
function safeLabel(value, fallback, maxLength = 80) {
    const text = trimPreview(String(value || '').replace(/\s+/g, ' '), maxLength);
    return text || fallback;
}

/**
 * @param {unknown} level
 */
function normalizeLogLevel(level) {
    return ['debug', 'info', 'warn', 'error'].includes(String(level || ''))
        ? /** @type {'debug' | 'info' | 'warn' | 'error'} */ (String(level))
        : 'info';
}

/**
 * @param {unknown} value
 * @param {number} depth
 */
function sanitizeLogDetail(value, depth = 0) {
    if (depth > 4) {
        return '[depth-limit]';
    }
    if (value == null || typeof value === 'number' || typeof value === 'boolean') {
        return value;
    }
    if (typeof value === 'string') {
        return trimPreview(value.replace(SECRET_VALUE_RE, '[redacted]'), 500);
    }
    if (Array.isArray(value)) {
        return value.slice(0, 20).map(item => sanitizeLogDetail(item, depth + 1));
    }
    if (typeof value === 'object') {
        const output = /** @type {Record<string, unknown>} */ ({});
        for (const [key, item] of Object.entries(/** @type {Record<string, unknown>} */ (value)).slice(0, 80)) {
            output[key] = SECRET_KEY_RE.test(key) ? '[redacted]' : sanitizeLogDetail(item, depth + 1);
        }
        return output;
    }
    return trimPreview(String(value), 160);
}

/**
 * @param {HTMLElement | null | undefined} element
 * @param {string} name
 */
function readCssPx(element, name) {
    if (!(element instanceof HTMLElement)) {
        return 0;
    }

    try {
        const raw = getComputedStyle(element).getPropertyValue(name);
        return Math.max(0, finiteNumber(Number.parseFloat(String(raw || '').trim()), 0));
    } catch {
        return 0;
    }
}

function readSafeInsets() {
    const root = document?.documentElement;
    return {
        top: readCssPx(root, SAFE_INSET_VARS.top),
        right: readCssPx(root, SAFE_INSET_VARS.right),
        bottom: readCssPx(root, SAFE_INSET_VARS.bottom),
        left: readCssPx(root, SAFE_INSET_VARS.left),
    };
}

function readVisualViewport() {
    const viewport = window?.visualViewport;
    if (!viewport) {
        return null;
    }

    return {
        width: finiteNumber(viewport.width),
        height: finiteNumber(viewport.height),
        offsetLeft: finiteNumber(viewport.offsetLeft),
        offsetTop: finiteNumber(viewport.offsetTop),
        scale: finiteNumber(viewport.scale, 1),
    };
}

function readScreen() {
    const screen = window?.screen;
    if (!screen) {
        return null;
    }

    return {
        width: finiteNumber(screen.width),
        height: finiteNumber(screen.height),
        availWidth: finiteNumber(screen.availWidth),
        availHeight: finiteNumber(screen.availHeight),
    };
}

function readPlatformSnapshot() {
    const nav = typeof navigator !== 'undefined' ? navigator : null;
    const userAgent = String(nav?.userAgent || '');
    const platform = String(nav?.platform || '');
    const maxTouchPoints = finiteNumber(nav?.maxTouchPoints);

    return {
        userAgent: trimPreview(userAgent, 320),
        platform: trimPreview(platform, 120),
        language: trimPreview(String(nav?.language || ''), 40),
        maxTouchPoints,
        android: /android/i.test(userAgent),
        ios: /iphone|ipad|ipod/i.test(userAgent)
            || (maxTouchPoints > 1 && (platform === 'MacIntel' || /macintosh/i.test(userAgent))),
    };
}

function readLocationSnapshot() {
    const location = window?.location;
    return {
        origin: String(location?.origin || ''),
        pathname: String(location?.pathname || ''),
    };
}

/**
 * @param {Element | null | undefined} element
 */
function describeElement(element) {
    if (!(element instanceof Element)) {
        return null;
    }

    const className = typeof element.className === 'string' ? element.className : '';
    return {
        tagName: String(element.tagName || '').toLowerCase(),
        id: trimPreview(String(element.id || ''), 80),
        className: trimPreview(className, 160),
        mobileSurface: trimPreview(String(element.getAttribute('data-tt-mobile-surface') || ''), 80),
        imeSurface: trimPreview(String(element.getAttribute('data-tt-ime-surface') || ''), 80),
    };
}

function readDocumentSnapshot() {
    return {
        readyState: String(document?.readyState || ''),
        visibilityState: String(document?.visibilityState || ''),
        activeElement: describeElement(document?.activeElement),
        location: readLocationSnapshot(),
    };
}

function readRuntimeSnapshot() {
    const hostWindow = /** @type {any} */ (window);
    let embeddedRuntimeProfile = null;
    try {
        embeddedRuntimeProfile = globalThis.localStorage?.getItem('tt:embeddedRuntimeProfile') ?? null;
    } catch {
        embeddedRuntimeProfile = null;
    }

    return {
        mobileRuntimeCompat: hostWindow.__TAURITAVERN_MOBILE_RUNTIME_COMPAT__ === true,
        overlayCompat: Boolean(hostWindow.__TAURITAVERN_MOBILE_OVERLAY_COMPAT__),
        embeddedRuntimeProfile,
        mobileSurfaceCount: document?.querySelectorAll?.('[data-tt-mobile-surface]')?.length ?? 0,
    };
}

/**
 * @param {unknown} reason
 */
function normalizeReason(reason) {
    const text = trimPreview(String(reason || '').trim(), 80);
    return text || 'manual';
}

/**
 * @param {ReturnType<readMobileDebugSnapshot>} snapshot
 */
function buildLogPayload(snapshot) {
    return {
        version: snapshot.version,
        reason: snapshot.reason,
        timestampMs: snapshot.timestampMs,
        platform: {
            android: snapshot.platform.android,
            ios: snapshot.platform.ios,
            userAgent: snapshot.platform.userAgent,
            maxTouchPoints: snapshot.platform.maxTouchPoints,
        },
        viewport: snapshot.viewport,
        layout: snapshot.layout,
        runtime: snapshot.runtime,
        document: snapshot.document,
        host: snapshot.host,
    };
}

/**
 * @param {{ reason?: unknown; now?: () => number }} [options]
 */
function readMobileDebugSnapshot(options = {}) {
    const hostWindow = /** @type {any} */ (window);
    const now = typeof options.now === 'function' ? options.now : Date.now;

    return {
        version: MOBILE_DEBUG_SNAPSHOT_VERSION,
        timestampMs: now(),
        reason: normalizeReason(options.reason),
        platform: readPlatformSnapshot(),
        viewport: {
            innerWidth: finiteNumber(window?.innerWidth),
            innerHeight: finiteNumber(window?.innerHeight),
            devicePixelRatio: finiteNumber(window?.devicePixelRatio, 1),
            visualViewport: readVisualViewport(),
            screen: readScreen(),
        },
        layout: {
            safeInsets: readSafeInsets(),
        },
        runtime: readRuntimeSnapshot(),
        document: readDocumentSnapshot(),
        host: {
            abiVersion: finiteNumber(hostWindow.__TAURITAVERN__?.abiVersion),
        },
    };
}

/**
 * @param {{
 *   now?: () => number;
 *   appendFrontendLogEntry?: (level: 'debug' | 'info' | 'warn' | 'error', message: string, target?: string) => void;
 * }} [deps]
 */
export function createMobileDebugApi(deps = {}) {
    const now = typeof deps.now === 'function' ? deps.now : Date.now;
    const appendFrontendLogEntry = typeof deps.appendFrontendLogEntry === 'function'
        ? deps.appendFrontendLogEntry
        : null;

    return {
        snapshot(options = {}) {
            return readMobileDebugSnapshot({
                ...options,
                now,
            });
        },
        async logSnapshot(options = {}) {
            const snapshot = readMobileDebugSnapshot({
                ...options,
                now,
            });
            const payload = JSON.stringify(buildLogPayload(snapshot));
            appendFrontendLogEntry?.(
                'info',
                `[TauriTavern][mobile-debug] snapshot ${payload}`,
                MOBILE_DEBUG_LOG_TARGET,
            );
            return snapshot;
        },
        async logEntry(options = {}) {
            const level = normalizeLogLevel(options?.level);
            const source = safeLabel(options?.source, 'extension');
            const event = safeLabel(options?.event, 'event', 120);
            const entry = {
                version: MOBILE_DEBUG_EXTENSION_LOG_VERSION,
                timestampMs: now(),
                source,
                event,
                detail: sanitizeLogDetail(options?.detail ?? null),
            };
            appendFrontendLogEntry?.(
                level,
                `[TauriTavern][mobile-debug][${source}] ${event} ${JSON.stringify(entry)}`,
                MOBILE_DEBUG_LOG_TARGET,
            );
            return { ok: true, timestampMs: entry.timestampMs };
        },
    };
}
