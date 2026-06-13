// Pure, dependency-injected message-identity stamping. No heavy imports so it is
// unit-testable under `node --test`. App wiring lives in ./message-identity.js.

/**
 * @param {{ uuid: () => string, hash: (s: string) => string }} deps
 */
export function makeStamper({ uuid, hash }) {
    function stampMessage(message) {
        if (!message || typeof message !== 'object') return message;
        if (!message.extra || typeof message.extra !== 'object') message.extra = {};
        const prev = message.extra.tauritavern;
        const meta = prev && typeof prev === 'object' ? prev : {};
        if (!meta.msgId) meta.msgId = uuid();
        meta.contentSha = hash(String(message.mes ?? ''));
        message.extra.tauritavern = meta;
        return message;
    }

    /** Stamp every message lacking an id. Returns count newly stamped. */
    function stampAll(messages) {
        let newly = 0;
        for (const m of messages ?? []) {
            const had = m && m.extra && m.extra.tauritavern && m.extra.tauritavern.msgId;
            stampMessage(m);
            if (!had) newly++;
        }
        return newly;
    }

    return { stampMessage, stampAll };
}
