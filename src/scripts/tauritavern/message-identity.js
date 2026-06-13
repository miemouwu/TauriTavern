// App entrypoint: wires the pure core with TauriTavern's uuid + sha256.
import { uuidv4 } from '../utils.js';
import { sha256 } from '../../lib.js';
import { makeStamper } from './message-identity-core.js';

const stamper = makeStamper({ uuid: uuidv4, hash: (s) => sha256(s) });

export const stampMessage = stamper.stampMessage;
export const stampAllMessages = stamper.stampAll;
