import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

class CssStyleDeclarationMock {
    #values = new Map();

    getPropertyValue(name) {
        return this.#values.get(String(name)) ?? '';
    }

    setProperty(name, value) {
        this.#values.set(String(name), String(value));
    }
}

class ElementMock {
    constructor(tagName = 'div') {
        this.tagName = String(tagName).toUpperCase();
        this.id = '';
        this.className = '';
        this.style = new CssStyleDeclarationMock();
        this.children = [];
        this.parentElement = null;
        this.#attrs = new Map();
    }

    #attrs;

    setAttribute(name, value) {
        this.#attrs.set(String(name), String(value));
    }

    getAttribute(name) {
        return this.#attrs.get(String(name)) ?? null;
    }

    hasAttribute(name) {
        return this.#attrs.has(String(name));
    }

    appendChild(child) {
        child.parentElement = this;
        this.children.push(child);
        return child;
    }

    querySelectorAll(selector) {
        const expected = String(selector).trim();
        const result = [];

        const visit = (node) => {
            for (const child of node.children) {
                if (expected === '[data-tt-mobile-surface]' && child.hasAttribute('data-tt-mobile-surface')) {
                    result.push(child);
                }
                visit(child);
            }
        };

        visit(this);
        return result;
    }
}

class HTMLElementMock extends ElementMock {}

function installMobileHarness() {
    globalThis.Element = ElementMock;
    globalThis.HTMLElement = HTMLElementMock;
    globalThis.getComputedStyle = (element) => element.style;

    const documentElement = new HTMLElementMock('html');
    documentElement.style.setProperty('--tt-inset-top', '24px');
    documentElement.style.setProperty('--tt-inset-right', '0px');
    documentElement.style.setProperty('--tt-inset-bottom', '16px');
    documentElement.style.setProperty('--tt-inset-left', '0px');

    const body = new HTMLElementMock('body');
    const dialogSurface = new HTMLElementMock('div');
    dialogSurface.id = 'mobile-popup';
    dialogSurface.className = 'drawer surface';
    dialogSurface.setAttribute('data-tt-mobile-surface', 'fullscreen-window');
    body.appendChild(dialogSurface);

    const composerSurface = new HTMLElementMock('textarea');
    composerSurface.id = 'send_textarea';
    composerSurface.setAttribute('data-tt-mobile-surface', 'viewport-host');
    body.appendChild(composerSurface);

    const documentMock = {
        documentElement,
        body,
        activeElement: composerSurface,
        readyState: 'complete',
        visibilityState: 'visible',
        querySelectorAll(selector) {
            if (String(selector).trim() === '[data-tt-mobile-surface]') {
                return body.querySelectorAll(selector);
            }
            return [];
        },
    };

    Object.defineProperty(globalThis, 'navigator', {
        value: {
            userAgent: 'Mozilla/5.0 (Linux; Android 16; PJZ110) AppleWebKit/537.36',
            platform: 'Linux armv8l',
            language: 'zh-CN',
            maxTouchPoints: 5,
        },
        configurable: true,
    });

    const windowMock = {
        innerWidth: 390,
        innerHeight: 844,
        devicePixelRatio: 3,
        visualViewport: {
            width: 390,
            height: 520,
            offsetLeft: 0,
            offsetTop: 0,
            scale: 1,
        },
        screen: {
            width: 390,
            height: 844,
            availWidth: 390,
            availHeight: 820,
        },
        location: {
            origin: 'https://tauri.localhost',
            pathname: '/index.html',
            search: '?secret=sk-do-not-log',
            hash: '#token=meow_do_not_log',
        },
        __TAURITAVERN__: {
            abiVersion: 1,
            api: {},
        },
        __TAURITAVERN_MOBILE_RUNTIME_COMPAT__: true,
        __TAURITAVERN_MOBILE_OVERLAY_COMPAT__: { active: true },
    };

    globalThis.document = documentMock;
    globalThis.window = windowMock;
    globalThis.localStorage = {
        getItem(key) {
            return key === 'tt:embeddedRuntimeProfile' ? 'mobile-safe' : null;
        },
    };

    return { windowMock };
}

test('api.dev.mobile captures a mobile runtime snapshot and writes it to frontend logs', async () => {
    installMobileHarness();

    const entries = [];
    const moduleUrl = pathToFileURL(path.join(REPO_ROOT, 'src/tauri/main/api/dev-mobile-debug.js'));
    const { createMobileDebugApi } = await import(`${moduleUrl.href}?test=${Date.now()}`);

    const api = createMobileDebugApi({
        now: () => 123456789,
        appendFrontendLogEntry(level, message, target) {
            entries.push({ level, message, target });
        },
    });

    const snapshot = api.snapshot({ reason: 'manual' });
    assert.equal(snapshot.version, 1);
    assert.equal(snapshot.reason, 'manual');
    assert.equal(snapshot.timestampMs, 123456789);
    assert.equal(snapshot.platform.android, true);
    assert.equal(snapshot.platform.ios, false);
    assert.equal(snapshot.viewport.innerWidth, 390);
    assert.equal(snapshot.viewport.visualViewport.height, 520);
    assert.equal(snapshot.layout.safeInsets.top, 24);
    assert.equal(snapshot.runtime.mobileSurfaceCount, 2);
    assert.equal(snapshot.document.activeElement.id, 'send_textarea');
    assert.deepEqual(snapshot.document.location, {
        origin: 'https://tauri.localhost',
        pathname: '/index.html',
    });

    const loggedSnapshot = await api.logSnapshot({ reason: 'manual' });
    assert.equal(loggedSnapshot.reason, 'manual');
    assert.equal(entries.length, 1);
    assert.deepEqual(entries[0], {
        level: 'info',
        target: 'mobile-debug',
        message: entries[0].message,
    });
    assert.match(entries[0].message, /^\[TauriTavern\]\[mobile-debug\] snapshot /);
    assert.match(entries[0].message, /"android":true/);
    assert.doesNotMatch(entries[0].message, /sk-do-not-log|meow_do_not_log/);
});

test('version extension exposes a mobile debug snapshot entry', async () => {
    const [settingsHtml, indexSource, zhCn, en] = await Promise.all([
        readFile(path.join(REPO_ROOT, 'src/scripts/extensions/tauritavern-version/settings.html'), 'utf8'),
        readFile(path.join(REPO_ROOT, 'src/scripts/extensions/tauritavern-version/index.js'), 'utf8'),
        readFile(path.join(REPO_ROOT, 'src/scripts/extensions/tauritavern-version/locales/zh-cn.json'), 'utf8'),
        readFile(path.join(REPO_ROOT, 'src/scripts/extensions/tauritavern-version/locales/en.json'), 'utf8'),
    ]);

    assert.match(settingsHtml, /id="tauritavern_mobile_debug_snapshot"/);
    assert.match(settingsHtml, /data-i18n="ttv_version\.mobile_debug_snapshot"/);
    assert.match(indexSource, /function\s+buildMobileDebugSnapshotContent/);
    assert.match(indexSource, /function\s+onMobileDebugSnapshotClick/);
    assert.match(indexSource, /devApi\.mobile\.logSnapshot/);
    assert.match(indexSource, /#tauritavern_mobile_debug_snapshot/);
    assert.match(zhCn, /"ttv_version\.mobile_debug_snapshot"/);
    assert.match(zhCn, /"ttv_version\.mobile_debug_snapshot_title"/);
    assert.match(en, /"ttv_version\.mobile_debug_snapshot"/);
    assert.match(en, /"ttv_version\.mobile_debug_snapshot_title"/);
});
