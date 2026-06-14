import { test } from 'node:test';
import assert from 'node:assert/strict';
import { makeStamper } from '../src/scripts/tauritavern/message-identity-core.js';

test('a legacy chat array gains stable ids that survive a re-stamp (re-save)', () => {
    const stamper = makeStamper({ uuid: (() => { let n = 0; return () => `uuid-${n++}`; })(), hash: (s) => `h(${s})` });
    const chat = [{ mes: 'old1', extra: {} }, { mes: 'old2', extra: {} }];
    stamper.stampAll(chat);              // first save
    const ids = chat.map(m => m.extra.tauritavern.msgId);
    assert.deepEqual(ids, ['uuid-0', 'uuid-1']);
    stamper.stampAll(chat);              // second save must NOT reassign
    assert.deepEqual(chat.map(m => m.extra.tauritavern.msgId), ids);
});
