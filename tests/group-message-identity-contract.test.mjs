import { test } from 'node:test';
import assert from 'node:assert/strict';
import { makeStamper } from '../src/scripts/tauritavern/message-identity-core.js';

test('group-chat save stamps every message before building the payload', () => {
    const stamper = makeStamper({ uuid: (() => { let n = 0; return () => `g-${n++}`; })(), hash: (s) => `h(${s})` });
    const chatHeader = { chat_metadata: {} };
    const chat = [{ mes: 'g1', extra: {} }, { mes: 'g2', extra: {} }];
    stamper.stampAll(chat);                 // what saveGroupChatUnsafe now does before payload
    const payload = [chatHeader, ...chat];
    assert.equal(payload[1].extra.tauritavern.msgId, 'g-0');
    assert.equal(payload[2].extra.tauritavern.msgId, 'g-1');
    stamper.stampAll(chat);                 // re-save must not reassign
    assert.equal(payload[1].extra.tauritavern.msgId, 'g-0');
});
