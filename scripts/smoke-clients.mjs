// The client half of scripts/smoke.sh.
//
// Behaves like a handful of browsers rather than a load generator: people
// arrive, talk, react, one of them reconnects, and everyone leaves. The point
// is to produce a *realistic log*, which the shell script then reads.

const PORT = process.env.PORT ?? '3199';
const ROOM = 'smoke';
const wait = (ms) => new Promise((r) => setTimeout(r, ms));

/** Opens a socket, optionally carrying cookies the way a returning browser does. */
function open(cookieHeader) {
  return new Promise((resolve, reject) => {
    const options = cookieHeader ? { headers: { Cookie: cookieHeader } } : undefined;
    const ws = new WebSocket(`ws://localhost:${PORT}/ws/${ROOM}`, options);
    const events = [];
    ws.addEventListener('message', (e) => events.push(JSON.parse(e.data)));
    ws.addEventListener('open', () => resolve({ ws, events }));
    ws.addEventListener('error', reject);
    setTimeout(() => reject(new Error('connect timed out')), 8000);
  });
}

/** Connects, and says plainly what happened if it cannot. */
async function join(label, cookieHeader) {
  try {
    return await open(cookieHeader);
  } catch (e) {
    console.error(`  ✘ ${label} could not connect: ${e.message ?? e.type ?? e}`);
    console.error('    A refusal here usually means slots were leaked: the');
    console.error('    per-address limit is small, so a handful of sessions that');
    console.error('    never released will refuse everything (constraint #1).');
    process.exit(1);
  }
}

const people = [];
for (let i = 0; i < 3; i += 1) {
  people.push(await join(`client ${i + 1}`));
  await wait(150);
}
await wait(400);

const names = people.map((p) => p.events.find((e) => e.type === 'Welcome')?.animal_name);
console.log('  joined as:', names.join(', '));

// Conversation: messages, a reply, reactions.
people[0].ws.send(JSON.stringify({ type: 'Message', text: 'morning' }));
await wait(200);
const first = people[1].events.find((e) => e.type === 'Message')?.message;
people[1].ws.send(JSON.stringify({
  type: 'Message',
  text: 'morning back',
  reply_to: { message_id: first.message_id, author_name: names[0], preview_text: 'morning' },
}));
await wait(200);
for (const emoji of ['👍', '🔥']) {
  people[2].ws.send(JSON.stringify({ type: 'React', message_id: first.message_id, emoji }));
  await wait(120);
}

// Typing, which is the event class that used to spend the message budget.
for (const typing of [true, false, true, false]) {
  people[0].ws.send(JSON.stringify({ type: 'Typing', is_typing: typing }));
  await wait(80);
}

// Something the server should refuse, so the refusal path is exercised.
people[0].ws.send(JSON.stringify({ type: 'React', message_id: crypto.randomUUID(), emoji: '🔥' }));
people[0].ws.send(JSON.stringify({ type: 'Message', text: '' }));
await wait(300);

// One person's connection blips and comes back. Without cookies this is the
// churn the log check looks for; the server sets them on the handshake, and a
// browser would send them back.
people[2].ws.close();
await wait(400);
people.push(await join('the reconnecting client'));
await wait(400);

console.log('  messages seen by the newest client:',
  people.at(-1).events.filter((e) => e.type === 'Message').length);

for (const p of people) {
  try { p.ws.close(); } catch { /* already closed */ }
}
await wait(600);
console.log('  clients closed');
