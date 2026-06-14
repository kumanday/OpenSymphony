const { WebSocket } = require('ws');
const { randomUUID } = require('crypto');
Object.assign(globalThis, { WebSocket });
if (typeof globalThis.crypto === 'undefined') {
  Object.assign(globalThis, { crypto: { randomUUID } });
} else if (typeof globalThis.crypto.randomUUID !== 'function') {
  globalThis.crypto.randomUUID = randomUUID;
}
