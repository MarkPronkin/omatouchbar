// Exercise the live persistence and input protocol against an isolated shell.json.
// Omarchy supplies the same place, people, solar and calendar models as on the bar.
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const { spawn } = require('node:child_process');
const { createInterface } = require('node:readline');
const source = fs.readFileSync(path.join(__dirname, '../src/world_clock.js'), 'utf8');

function controller(t, widget, preview = false) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'omatouchbar-clock-'));
  const shell = path.join(dir, 'omarchy/shell.json');
  fs.mkdirSync(path.dirname(shell));
  const write = (entries) =>
    fs.writeFileSync(shell, JSON.stringify({ bar: { layout: { center: entries } } }));
  write([widget, { id: 'omarchy.clock' }]);
  const read = () => JSON.parse(fs.readFileSync(shell)).bar.layout.center;
  fs.mkdirSync(path.join(dir, 'bin'));
  const executable = (name, source) =>
    fs.writeFileSync(path.join(dir, 'bin', name), source, { mode: 0o755 });
  // All external effects, including network and changing the host timezone,
  // are confined to the fixture, even when running the live controller.
  executable(
    'omarchy-shell',
    `#!/usr/bin/env node
const fs=require('node:fs'),path=process.env.XDG_CONFIG_HOME+'/omarchy/shell.json';
const [target,method,id,key,value,selector]=process.argv.slice(2);
if(target!=='shell'||method!=='setBarWidget'||selector!=='{}')process.exit(1);
const config=JSON.parse(fs.readFileSync(path));
config.bar.layout.center.find(c=>c.id===id)[key]=JSON.parse(value);
fs.writeFileSync(path,JSON.stringify(config)); console.log('ok');
`,
  );
  executable(
    'curl',
    `#!/bin/sh
case "$*" in
*geocoding-api*) echo '{"results":[{"name":"Test Harbor","timezone":"America/New_York","latitude":27.2,"longitude":-80.2,"country_code":"US","admin1":"Florida"}]}' ;;
*) exit 1 ;;
esac
`,
  );
  executable('systemd-run', '#!/bin/sh\nexit 1\n');
  const child = spawn(process.execPath, ['-e', source, preview ? 'preview' : 'live'], {
    env: {
      ...process.env,
      HOME: dir,
      XDG_CONFIG_HOME: dir,
      XDG_CACHE_HOME: path.join(dir, 'cache'),
      PATH: path.join(dir, 'bin') + ':' + process.env.PATH,
    },
    stdio: ['pipe', 'pipe', 'inherit'],
  });
  const frames = createInterface({ input: child.stdout })[Symbol.asyncIterator]();
  t.after(() => {
    child.kill();
    fs.rmSync(dir, { recursive: true, force: true });
  });
  async function wait(check) {
    for (;;) {
      const { value, done } = await frames.next();
      assert.equal(done, false, 'clock worker exited');
      const frame = JSON.parse(value);
      assert.ok(!frame.error, frame.error);
      const error = frame.items.find((i) => i.id === 'notice');
      if (error && !check(frame)) assert.fail(error.title);
      if (check(frame)) return frame;
    }
  }
  async function send(op, value, check) {
    child.stdin.write(JSON.stringify({ op, value, session: 1 }) + '\n');
    return wait(check);
  }
  return { read, write, wait, send, tap: (id, check) => send('tap', id, check) };
}
const view = (name) => (f) => f.view === name;
const item = (frame, id) => frame.items.find((i) => i.id === id);
test('city search, people, shared-zone identity and durable edits', { timeout: 15000 }, async (t) => {
  const fixture = {
    id: 'omarchy.elsewhen',
    hour24: true,
    cities: [
      {
        name: 'New York',
        zone: 'America/New_York',
        people: ['Jo'],
        lat: 40.7,
        lon: -74,
        region: 'NY',
        custom: 7,
      },
      { name: 'Miami', zone: 'America/New_York', people: ['Bo'] },
    ],
  };
  const { read, write, send, tap, wait } = controller(t, fixture);
  await send('open', null, view('list'));
  await tap('add', view('add'));
  await send('text', 'Test Harbor', (f) => item(f, 'field')?.title === 'Test Harbor');
  const found = await wait((f) =>
    f.items.some((i) => i.title === 'Test Harbor' && i.sub === 'Florida, US'),
  );
  await tap(found.items.find((i) => i.sub === 'Florida, US').id, view('list'));
  assert.equal(read()[0].cities[2].lat, 27.2);
  await tap('choosePeople', view('choosePeople'));
  await tap('who/Miami|America/New_York', (f) => f.editing);
  await send('clear', '', (f) => item(f, 'field')?.title.startsWith('Names'));
  await send('text', 'Ana, @octo, ana', (f) => item(f, 'field')?.title === 'Ana, @octo, ana');
  // Another surface edits a different city between opening the editor and saving.
  const entries = read();
  entries[0].cities[0].people = ['Desktop edit'];
  write(entries);
  await send('enter', null, view('city'));
  assert.deepEqual(read()[0].cities[1].people, ['Ana', '@octo']);
  assert.deepEqual(read()[0].cities[0], { ...fixture.cities[0], people: ['Desktop edit'] });
  await tap('earlierCity', (f) => f.view === 'city' && !item(f, 'earlierCity'));
  assert.equal(read()[0].cities[0].name, 'Miami');
  await tap('editPeople', (f) => f.editing);
  await send('clear', '', (f) => item(f, 'field')?.title.startsWith('Names'));
  await send('back', null, view('city'));
  assert.deepEqual(read()[0].cities[0].people, ['Ana', '@octo']);
  await tap('remove', view('list'));
  assert.deepEqual(
    read()[0].cities.map((c) => c.name),
    ['New York', 'Test Harbor'],
  );
  await tap('city/Test Harbor|America/New_York', view('city'));
  await tap('remove', view('list'));
  const last = await tap('city/New York|America/New_York', view('city'));
  assert.equal(item(last, 'remove'), undefined);
});

test(
  'legacy list, offline aliases, globe and calendar work in the read-only preview',
  { timeout: 8000 },
  async (t) => {
    const original = { id: 'omarchy.elsewhen', zones: 'Paris|Europe/Paris, Tokyo|Asia/Tokyo' };
    const { read, send, tap } = controller(t, original, true);
    const list = await send('open', null, view('list'));
    assert.ok(item(list, 'city/Paris|Europe/Paris'));
    await tap('add', view('add'));
    const matches = await send('text', 'Miami', (f) => item(f, 'field')?.title === 'Miami');
    assert.equal(matches.items.find((i) => i.id.startsWith('pick/')).sub, 'America/New_York');
    await send('enter', null, view('list'));
    await tap('city/Miami|America/New_York', view('city'));
    await tap('editPeople', (f) => f.editing);
    await send('text', 'Ada', (f) => item(f, 'field')?.title === 'Ada');
    const detail = await send('enter', null, view('city'));
    assert.equal(item(detail, 'editPeople').title, 'Ada');
    assert.ok(item(detail, 'daylight').drag);
    await tap('later', (f) => item(f, 'now')?.sub === 'Return to now');
    const globe = await tap('globe', view('globe'));
    assert.ok(item(globe, 'earth').land.length && item(globe, 'moon'));
    await tap('jump', view('jump'));
    await send('text', 'Tokyo', (f) => item(f, 'field')?.title === 'Tokyo');
    await send('enter', null, (f) => item(f, 'earth')?.title === 'Tokyo');
    await send('back', null, view('list'));
    const cal = await tap('calendar', view('calendar'));
    assert.equal(cal.items.flatMap((i) => i.week || []).length, 42);
    assert.equal(cal.items.flatMap((i) => i.week || []).filter((d) => d.today).length, 1);
    const title = item(cal, 'month').title;
    await tap('nextYear', (f) => item(f, 'month')?.title !== title);
    await tap('today', (f) => item(f, 'month')?.title === title);
    await tap('yearMeter', view('birth'));
    await send('text', '1990', (f) => item(f, 'field')?.title === '1990');
    await send('enter', null, view('span'));
    await send('text', '95', (f) => item(f, 'field')?.title === '95');
    await send('enter', null, (f) => item(f, 'lifeMeter'));
    assert.deepEqual(read()[0], original);
  },
);
