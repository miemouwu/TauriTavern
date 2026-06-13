import { test } from 'node:test';
import assert from 'node:assert/strict';
import { makeStamper } from '../src/scripts/tauritavern/message-identity-core.js';

const stamper = makeStamper({
    uuid: () => 'uuid-fixed',
    hash: (s) => `h(${s})`,
});

test('stamps a fresh message with msgId + contentSha', () => {
    const m = { mes: 'hi', extra: {} };
    stamper.stampMessage(m);
    assert.equal(m.extra.tauritavern.msgId, 'uuid-fixed');
    assert.equal(m.extra.tauritavern.contentSha, 'h(hi)');
});

test('msgId is idempotent; contentSha refreshes on edit', () => {
    const m = { mes: 'a', extra: { tauritavern: { msgId: 'keep' } } };
    stamper.stampMessage(m);
    assert.equal(m.extra.tauritavern.msgId, 'keep');
    m.mes = 'b';
    stamper.stampMessage(m);
    assert.equal(m.extra.tauritavern.msgId, 'keep');
    assert.equal(m.extra.tauritavern.contentSha, 'h(b)');
});

test('stampAll backfills only unstamped and returns the new count', () => {
    const msgs = [
        { mes: 'x', extra: { tauritavern: { msgId: 'old' } } },
        { mes: 'y', extra: {} },
        { mes: 'z' },
    ];
    const n = stamper.stampAll(msgs);
    assert.equal(n, 2);
    assert.equal(msgs[0].extra.tauritavern.msgId, 'old');
    assert.equal(msgs[1].extra.tauritavern.msgId, 'uuid-fixed');
    assert.equal(msgs[2].extra.tauritavern.msgId, 'uuid-fixed');
});
