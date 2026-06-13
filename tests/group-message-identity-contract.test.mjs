import { test } from 'node:test';
import assert from 'node:assert/strict';
import { makeStamper } from '../src/scripts/tauritavern/message-identity-core.js';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

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

test('group-chats.js actually calls stampAllMessages in its save + open paths (regression guard)', async () => {
    const src = await readFile(
        path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../src/scripts/group-chats.js'),
        'utf8',
    );
    assert.match(src, /import \{ stampAllMessages \} from '\.\/tauritavern\/message-identity\.js'/);
    const saveIdx = src.indexOf('async function saveGroupChatUnsafe');
    assert.ok(saveIdx >= 0, 'saveGroupChatUnsafe present');
    assert.match(src.slice(saveIdx, saveIdx + 1200), /stampAllMessages\(chat\)/);
    const openIdx = src.indexOf('export async function getGroupChat');
    assert.ok(openIdx >= 0, 'getGroupChat present');
    assert.match(src.slice(openIdx, openIdx + 6000), /stampAllMessages\(chat\)/);
});
