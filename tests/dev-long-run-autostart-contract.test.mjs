import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

test('Tauri debug builds can auto-start dev longRun from environment', async () => {
    const source = await readFile(path.join(REPO_ROOT, 'src-tauri/src/lib.rs'), 'utf8');

    assert.match(source, /#\[cfg\(debug_assertions\)\]\s*fn install_dev_long_run_autostart/);
    assert.match(source, /TAURITAVERN_DEV_LONGRUN_TURNS/);
    assert.match(source, /TAURITAVERN_DEV_LONGRUN_REPORT_URL/);
    assert.doesNotMatch(source, /waitForActiveChat/);
    assert.match(source, /waitForSillyTavernContext/);
    assert.match(source, /window\.SillyTavern\?\.getContext/);
    assert.match(source, /await waitForLongRun\(\);\s*await waitForSillyTavernContext\(\);\s*const report = await window\.__TAURITAVERN__\.api\.dev\.longRun\.start\(options\);/s);
    assert.match(source, /window\.__TAURITAVERN__\.api\.dev\.longRun\.start/);
    assert.match(source, /\.eval\(&script\)/);
    assert.match(source, /install_dev_long_run_autostart\(&_main_window\)/);
});
