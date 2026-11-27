// Touch-strip controller for Omarchy's installed Elsewhen and calendar models.
// stdin carries ordered scene events; stdout carries declarative strip contents.
'use strict';
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const readline = require('node:readline');
const { execFile } = require('node:child_process');
const run = require('node:util').promisify(execFile);
const preview = process.argv[1] === 'preview';
const root = process.env.OMARCHY_PATH || '/usr/share/omarchy';
const config = process.env.XDG_CONFIG_HOME || path.join(require('node:os').homedir(), '.config');
const panels = path.join(root, 'shell/plugins/panels');
const settingsFile = path.join(config, 'omarchy/shell.json');
const standaloneFile = path.join(config, 'omatouchbar/world-clock.json');

function json(file, fallback) {
  try {
    return JSON.parse(fs.readFileSync(file, 'utf8'));
  } catch (e) {
    if (e.code === 'ENOENT') return fallback;
    throw e;
  }
}
function model(file, imports = {}) {
  const code = fs.readFileSync(path.join(panels, file), 'utf8').replace(/^\.(pragma|import).*$/gm, '');
  const scope = vm.createContext({ ...imports });
  vm.runInContext(code, scope, { filename: file });
  return scope;
}
let M, Solar, Sun, Greetings, Calendar, builtIn, land, options, zoneTab;
function loadModels() {
  if (options) return;
  M = model('elsewhen/Model.js');
  Solar = model('elsewhen/GlobeModel.js');
  Sun = model('elsewhen/Sun.js', { Solar });
  Greetings = model('elsewhen/Greetings.js');
  Calendar = model('clock/Model.js');
  builtIn = json(path.join(panels, 'elsewhen/cities.json'), []);
  zoneTab = fs.readFileSync('/usr/share/zoneinfo/zone1970.tab', 'utf8');
  const zones = fs
    .readFileSync('/usr/share/zoneinfo/tzdata.zi', 'utf8')
    .split('\n')
    .flatMap((line) => (line.startsWith('Z ') ? [line.split(' ')[1]] : []));
  options = M.zoneOptions(zones.join('\n'), []);
}

let session = 0,
  page = 'list',
  focus = '',
  query = '',
  shift = 0,
  notice = '';
let replaceNumber = false;
let selection = 0,
  previewHome = '',
  retire;
let world, clock, cities, locale, homeId, hour24, units, here;
let jumpCity, confirmHere, spin, latitude, month, birthInput;
let actions = new Map(),
  items = [],
  tick,
  debounce,
  scrubReset;
const previewChanges = new Map();
const weather = new Map(),
  geo = new Map(),
  places = new Map(),
  profiles = {};
const triedProfiles = new Set(),
  pendingPlaces = new Set(),
  formatted = new Map();
const cityKey = (c) => M.factsKey(c);
const selected = () => cities.find((c) => cityKey(c) === focus);
const now = () => Date.now() + shift * 60000;
const editing = () => ['add', 'here', 'jump', 'people', 'birth', 'span'].includes(page);
const dateFmt = (ms, zone, opts) => {
  const key = JSON.stringify([locale, zone, opts]);
  if (!formatted.has(key))
    formatted.set(key, new Intl.DateTimeFormat(locale, { ...opts, timeZone: zone }));
  return formatted.get(key).format(ms);
};
function offset(zone, ms) {
  const fmt = new Intl.DateTimeFormat('en', { timeZone: zone, timeZoneName: 'longOffset' });
  const value = fmt.formatToParts(ms).find((p) => p.type === 'timeZoneName').value;
  const m = value.match(/([+-])(\d\d):(\d\d)/);
  return m ? (m[1] === '-' ? -1 : 1) * (+m[2] * 60 + +m[3]) : 0;
}
const time = (ms, id) =>
  dateFmt(ms, id, { hour: 'numeric', minute: '2-digit', hourCycle: hour24 ? 'h23' : 'h12' });
function widget(entries, id, fallback) {
  for (let entry of entries) {
    if (typeof entry === 'string') entry = { id: entry };
    if (!entry || typeof entry.id !== 'string') continue;
    if (
      entry.id === id ||
      (!entry.id.startsWith('omarchy.') &&
        /^[\w.-]+$/.test(entry.id) &&
        json(path.join(config, 'omarchy/plugins', entry.id, 'manifest.json'), {})?.omarchy
          ?.clonedFrom === id)
    )
      return { ...entry, ...previewChanges.get(entry.id) };
  }
  return { id: null, ...fallback[id], ...previewChanges.get(id) };
}
function readSettings() {
  const shell = json(settingsFile, null) ?? json(path.join(root, 'config/omarchy/shell.json'), {});
  const entries = Object.values(shell.bar?.layout || {}).flat();
  const standalone = json(standaloneFile, {});
  world = widget(entries, 'omarchy.elsewhen', standalone);
  clock = widget(entries, 'omarchy.clock', standalone);
  let language = 'en';
  try {
    language = fs.readFileSync(path.join(config, 'omarchy/language'), 'utf8').trim();
  } catch {}
  locale = json(path.join(root, 'default/i18n/locales.json'), {})[language]?.formatLocale || 'en-US';
  homeId = Intl.DateTimeFormat().resolvedOptions().timeZone;
  // ICU canonicalizes some IDs (e.g. Asia/Calcutta); Elsewhen uses the system's spelling.
  try {
    homeId = fs.realpathSync('/etc/localtime').split('/zoneinfo/')[1] || homeId;
  } catch {}
  if (previewHome) homeId = previewHome;
  here = M.resolveHere(world.here, world.homeCity, homeId);
  hour24 = M.resolveHour24(
    world.hour24,
    !new Intl.DateTimeFormat(locale, { hour: 'numeric' }).resolvedOptions().hour12,
  );
  units = M.resolveUnits(world.units, new Intl.Locale(locale).region === 'US' ? 'F' : 'C');
  cities = M.parseCities(world.cities, world.zones);
  if (!cities.length) {
    const offsets = {};
    for (const id of [homeId, ...M.seedCandidateZones()]) {
      try {
        offsets[id] = offset(id, Date.now());
      } catch {}
    }
    cities = M.seedZones(here, offsets, 4);
  }
}
async function persist(target, values) {
  const id = target.id || (target === world ? 'omarchy.elsewhen' : 'omarchy.clock');
  if (preview) {
    previewChanges.set(id, { ...previewChanges.get(id), ...values });
  } else if (target.id) {
    for (const [key, value] of Object.entries(values)) {
      const { stdout } = await run(
        'omarchy-shell',
        ['shell', 'setBarWidget', id, key, JSON.stringify(value), '{}'],
        { timeout: 8000 },
      );
      if (stdout.trim() !== 'ok') throw Error(stdout.trim() || 'Omarchy could not save the change');
    }
  } else {
    const stored = json(standaloneFile, {});
    stored[id] = { ...stored[id], ...values };
    fs.mkdirSync(path.dirname(standaloneFile), { recursive: true });
    const tmp = standaloneFile + '.' + process.pid;
    fs.writeFileSync(tmp, JSON.stringify(stored), { mode: 0o600 });
    fs.renameSync(tmp, standaloneFile);
  }
  readSettings();
  // Quickshell may acknowledge the change one event before its atomic
  // shell.json write lands. Wait for that write before accepting another edit.
  if (!preview && target.id) {
    const saved = () => (target.id === world.id ? world : clock);
    for (let attempt = 0; attempt < 20; attempt++) {
      if (Object.entries(values).every(([k, v]) => JSON.stringify(saved()[k]) === JSON.stringify(v)))
        return;
      await new Promise((resolve) => setTimeout(resolve, 25));
      readSettings();
    }
    throw Error('Omarchy did not save the change');
  }
}
// Re-read before mutating a list so edits on the desktop and two places in
// the same time zone keep their identities, order, coordinates and people.
function storedCities(next) {
  const saved = new Map(
    (Array.isArray(world.cities) ? world.cities : [])
      .filter((c) => c && typeof c.zone === 'string')
      .map((c) => [cityKey({ label: c.name, id: c.zone }), c]),
  );
  return M.serializeCities(next).map((city) => {
    const raw = saved.get(cityKey({ label: city.name, id: city.zone }));
    if (!raw) return city;
    const { people, lat, lon, region, ...rest } = raw;
    return { ...rest, ...city };
  });
}
async function editCities(transform) {
  readSettings();
  const next = transform(cities);
  if (next !== cities) await persist(world, { cities: storedCities(next) });
  refreshFacts();
}
function enter(next, text = '') {
  page = next;
  selection = 0;
  query = text;
  notice = '';
  replaceNumber = page === 'birth' || page === 'span';
  clearTimeout(debounce);
}
function back() {
  if (page === 'people') enter(selected() ? 'city' : 'list');
  else if (page === 'jump') enter('globe');
  else if (page === 'span' || page === 'birth') enter('calendar');
  else if (page === 'confirm') enter('here');
  else enter('list');
}
function add(id, title, sub, icon, action, extra = {}) {
  if (action) actions.set(id, action);
  items.push({ id, title, sub, icon, action: !!action, ...extra });
}
function focusCity(c) {
  focus = cityKey(c);
  enter('city');
}
function spot(c) {
  if (Number.isFinite(c.lat) && Number.isFinite(c.lon)) return c;
  const found = geo.get(cityKey(c));
  if (found) return found;
  const pin = builtIn.find((p) => p[0] === c.label && p[1] === c.id);
  if (pin) return { lat: pin[2], lon: pin[3] };
  return M.zoneTabCoords(zoneTab, c.id);
}
function row(c) {
  const ms = now(),
    off = offset(c.id, ms),
    parts = M.zoneParts(ms, off);
  const delta = M.dayDelta(parts, M.zoneParts(ms, offset(homeId, ms)));
  const words = M.dayLabel(delta);
  const zone =
    world.offsetMode === 'utc' ? M.utcOffsetLabel(off) : M.relativeOffsetLabel(off, offset(homeId, ms));
  const people = (c.people || []).map((p) => M.personName(p, profiles)).join(', ');
  const temp = weather.get(cityKey(c));
  return {
    ms,
    off,
    parts,
    people,
    zone,
    words,
    temperature: temp?.c == null ? '' : M.formatTemp(temp.c, units),
  };
}
function faces(c) {
  const people = (c.people || []).slice(0, 4);
  return {
    avatars: people.map((p) => profiles[M.personHandle(p)]?.avatar || ''),
    initials: people.map((p) => M.initialOf(M.personName(p, profiles))),
  };
}
function renderList() {
  add('here', time(now(), homeId) + ' · ' + here.label, 'Where are you?', '⌖', () =>
    startSearch('here'),
  );
  add('add', 'Add a city', '', '󰐕', () => startSearch('add'));
  if (shift) planning();
  add('choosePeople', 'People', '', '󰀎', () => enter('choosePeople'));
  for (const c of cities) {
    try {
      const r = row(c);
      add(
        'city/' + cityKey(c),
        c.label + '  ' + time(r.ms, c.id),
        [r.people, r.words, r.zone, r.temperature].filter(Boolean).join(' · ') ||
          dateFmt(r.ms, c.id, { weekday: 'long' }),
        '',
        () => focusCity(c),
        faces(c),
      );
    } catch {
      add('city/' + cityKey(c), c.label, 'Unknown time zone', '', () => focusCity(c));
    }
  }
  if (world.globeEnabled !== false) add('globe', 'Globe', '', '󰇧', () => showGlobe(here));
  add(
    'calendar',
    dateFmt(Date.now(), homeId, { month: 'short', day: 'numeric' }),
    'Calendar',
    '󰃭',
    openCalendar,
  );
  controls();
  add('refresh', 'Refresh', '', '󰑓', () => {
    readSettings();
    refreshFacts(true);
  });
}
function controls() {
  add('hour', hour24 ? '24-hour' : '12-hour', 'Time format', '󰥔', () =>
    persist(world, { hour24: !hour24 }),
  );
  add('offset', world.offsetMode === 'utc' ? 'UTC offsets' : 'From home', 'Time difference', '', () =>
    persist(world, { offsetMode: world.offsetMode === 'utc' ? 'home' : 'utc' }),
  );
  add('units', '°' + units, 'Temperature', '', () =>
    persist(world, { units: units === 'F' ? 'C' : 'F' }),
  );
}
function shiftTime(delta) {
  clearTimeout(scrubReset);
  shift += delta;
}
function planning() {
  add('earlier', '−1 h', '', '', () => shiftTime(-60));
  add('now', shift ? M.formatScrubDelta(shift) : 'Now', shift ? 'Return to now' : 'Live time', '', () =>
    shiftTime(-shift),
  );
  add('later', '+1 h', '', '', () => shiftTime(60));
}
function cityDetail(c) {
  const pos = M.indexOfZoneKey(cities, cityKey(c));
  if (c.people?.length) {
    for (const p of c.people) {
      const handle = M.personHandle(p);
      add(
        'person/' + p,
        M.personName(p, profiles),
        handle ? '@' + handle : 'Person',
        '',
        null,
        faces({ people: [p] }),
      );
    }
  }
  if (pos > 0) add('earlierCity', 'Move earlier', '', '󰅁', () => moveCity(c, -1));
  if (pos >= 0 && pos < cities.length - 1) add('laterCity', 'Move later', '', '󰅂', () => moveCity(c, 1));
  if (cities.length > 1 && pos >= 0)
    add('remove', 'Remove city', c.label, '󰆴', async () => {
      await editCities((z) => M.removeZoneAt(z, M.indexOfZoneKey(z, cityKey(c))));
      enter('list');
    });
}
async function moveCity(c, step) {
  await editCities((z) => {
    const at = M.indexOfZoneKey(z, cityKey(c));
    return M.moveZone(z, at, at + step);
  });
}
function renderCity(c) {
  let r;
  try {
    r = row(c);
  } catch {
    cityDetail(c);
    return;
  }
  const g = Greetings.greeting(c.id, r.parts.hour);
  add(
    'clock',
    time(r.ms, c.id) + ' · ' + c.label,
    [dateFmt(r.ms, c.id, { weekday: 'short', month: 'short', day: 'numeric' }), r.words, r.zone]
      .filter(Boolean)
      .join(' · '),
    '󰥔',
    () => persist(world, { hour24: !hour24 }),
  );
  add(
    'editPeople',
    r.people || 'Add people',
    r.people ? 'Edit people' : c.label,
    '󰀎',
    () => enter('people', M.peopleText(c.people)),
    faces(c),
  );
  const place = spot(c);
  if (place) {
    const sun = Sun.sunTimes(place.lat, place.lon, r.ms, r.off);
    const caption =
      sun.kind === 'normal'
        ? '↑ ' + time(sun.riseMs, c.id) + '   ↓ ' + time(sun.setMs, c.id)
        : sun.kind === 'polarNight'
          ? 'Polar night'
          : 'Midnight sun';
    add('daylight', 'Daylight', caption, '', null, {
      drag: true,
      fraction: (r.parts.hour * 60 + r.parts.minute) / 1440,
      daylight: Sun.litSpans(sun).map((s) => [s.x0, s.x1]),
    });
  }
  planning();
  add('greeting', g.text, [g.roman, g.language].filter(Boolean).join(' · '), '', null);
  const zoneName = new Intl.DateTimeFormat(locale, { timeZone: c.id, timeZoneName: 'short' })
    .formatToParts(r.ms)
    .find((p) => p.type === 'timeZoneName')?.value;
  add(
    'zone',
    zoneName || c.id,
    world.offsetMode === 'utc' ? M.utcOffsetLabel(r.off) : r.zone || 'Same as home',
    '',
    () => persist(world, { offsetMode: world.offsetMode === 'utc' ? 'home' : 'utc' }),
  );
  if (r.temperature)
    add('weather', r.temperature, M.weatherKind(weather.get(cityKey(c)).w), '', () =>
      persist(world, { units: units === 'C' ? 'F' : 'C' }),
    );
  if (world.globeEnabled !== false) add('globe', 'Globe', c.label, '󰇧', () => showGlobe(c));
  cityDetail(c);
}
function startSearch(target) {
  enter(target);
}
function matches() {
  const existing = page === 'add' ? cities : [];
  const available = options.filter((o) => !M.hasEntry(existing, o.value, o.label));
  const offline = M.searchZones(available, query, 8);
  return M.mergePlaces(offline, places.get(M.placeQuery(query)) || [], existing, 12);
}
function scheduleSearch() {
  clearTimeout(debounce);
  const q = M.placeQuery(query);
  if (preview || !q || pendingPlaces.has(q) || places.has(q)) return;
  debounce = setTimeout(async () => {
    pendingPlaces.add(q);
    try {
      const result = await fetchJson(M.geocodeUrl(q));
      places.set(q, M.placeOptions(result));
    } catch {
      places.set(q, []);
    } finally {
      pendingPlaces.delete(q);
    }
    if (session && ['add', 'here', 'jump'].includes(page) && M.placeQuery(query) === q) render();
  }, 300);
}
function searchView() {
  const prompt = { add: 'Add a city', here: 'Where are you?', jump: 'Jump to a city' }[page];
  add('field', query || 'Type a city or time zone', prompt, '󰍉', null, { active: true });
  add('erase', '⌫', '', '', () => editText('backspace'));
  const found = matches();
  if (!found.length)
    add(
      'empty',
      pendingPlaces.has(M.placeQuery(query)) ||
        (M.placeQuery(query) && !preview && !places.has(M.placeQuery(query)))
        ? 'Looking up places…'
        : 'No matching cities',
      '',
      '',
      null,
    );
  selection = Math.min(selection, Math.max(0, found.length - 1));
  for (const [index, o] of found.entries()) {
    const id = JSON.stringify([o.label, o.value, o.region || '', o.lat, o.lon]);
    add('pick/' + id, o.label, o.region || o.value, '', () => pick(o), { active: index === selection });
  }
}
async function pick(o) {
  const c = M.withPlace({ label: o.label, id: o.value }, o);
  if (page === 'jump') return showGlobe(c);
  if (page === 'here') {
    if (homeId !== c.id) {
      confirmHere = c;
      enter('confirm');
      return;
    }
    return setHere(c);
  }
  await editCities((z) => M.addZone(z, c.id, c.label, c));
  enter('list');
}
async function setHere(c) {
  readSettings();
  const next = M.hereAfterPick(cities, here, c, homeId);
  if (!next) return;
  if (next.zoneChange && !preview) {
    // The renderer has NoNewPrivileges. Run the same Omarchy timezone
    // command in a user service, and only save "here" after it succeeds.
    await run(
      'systemd-run',
      ['--user', '--quiet', '--wait', '--collect', '--', ...M.timezoneCommand(next.zoneChange)],
      { timeout: 15000 },
    );
  }
  if (preview && next.zoneChange) previewHome = next.zoneChange;
  const values = { here: next.here, homeCity: '' };
  if (next.zones !== cities) values.cities = storedCities(next.zones);
  await persist(world, values);
  enter('list');
  refreshFacts();
}
function showGlobe(c) {
  if (!land)
    land = json(path.join(panels, 'elsewhen/world.json'), []).map((r) => Solar.decimateRing(r, 3, 7));
  jumpCity = c;
  const pos = spot(c);
  spin = pos?.lon || 0;
  latitude = pos?.lat || 0;
  enter('globe');
}
function renderGlobe() {
  const c = jumpCity,
    r = row(c),
    pins = [here, ...cities, c];
  const xy = (p) => [Math.round(p.x * 1000) / 1000, Math.round(p.y * 1000) / 1000];
  const rings = land
    .map((r) => Solar.clipRingToDisc(r, spin, latitude, 1).map(xy))
    .filter((r) => r.length);
  const markers = pins
    .map(spot)
    .filter(Boolean)
    .map((p) => Solar.project(p.lat, p.lon, spin, latitude, 1))
    .filter((p) => p.visible)
    .map(xy);
  const sun = Solar.subsolarPoint(r.ms);
  const daylight = Solar.project(sun.lat, sun.lon, spin, latitude, 1);
  const light = [daylight.x, -daylight.y, daylight.cosc];
  add('earth', c.label, time(r.ms, c.id) + ' · ' + (r.zone || 'Home offset'), '', null, {
    land: rings,
    markers,
    light,
    drag: true,
  });
  add('jump', 'Jump to a city', '', '󰍉', () => startSearch('jump'));
  add('rotateWest', 'Rotate west', '', '󰅁', () => {
    spin -= 25;
  });
  add('rotateEast', 'Rotate east', '', '󰅂', () => {
    spin += 25;
  });
  if (M.indexOfZone(cities, c.label, c.id) < 0)
    add('track', 'Add city', c.label, '󰐕', () => editCities((z) => M.addZone(z, c.id, c.label, c)));
  if (c.id === homeId) add('claim', 'Set as home', c.label, '⌖', () => setHere(c));
  for (const pin of cities) {
    try {
      add('pin/' + cityKey(pin), pin.label, time(r.ms, pin.id), '', () => showGlobe(pin), {
        active: cityKey(pin) === cityKey(c),
      });
    } catch {
      /* An unknown saved zone is still removable from the city list. */
    }
  }
  planning();
  const phase = Solar.moonPhase(r.ms);
  const phases = [
    'New moon',
    'Waxing crescent',
    'First quarter',
    'Waxing gibbous',
    'Full moon',
    'Waning gibbous',
    'Last quarter',
    'Waning crescent',
  ];
  add(
    'moon',
    phases[Math.round(phase * 8) % 8],
    Math.round(50 * (1 - Math.cos(2 * Math.PI * phase))) + '% illuminated',
    '☾',
    null,
  );
  add('largeGlobe', 'Desktop globe', '', '󰍹', async () => {
    if (preview) return;
    const id = world.id || 'omarchy.elsewhen';
    await run('omarchy-shell', ['shell', 'summon', id]);
    await run('omarchy-shell', [id, 'globe']);
  });
}
function openCalendar() {
  month = null;
  enter('calendar');
}
function calendarView() {
  const d = new Date(),
    y = d.getFullYear(),
    m = d.getMonth(),
    day = d.getDate();
  const start = Calendar.normalizedWeekStart(
    clock.weekStartDay,
    (new Intl.Locale(locale).getWeekInfo?.() || new Intl.Locale(locale).weekInfo || { firstDay: 7 })
      .firstDay % 7,
  );
  const v = month || { year: y, month: m };
  const step = (delta) => {
    month = Calendar.stepMonth(v.year, v.month, delta);
    if (month.year === y && month.month === m) month = null;
  };
  add(
    'today',
    dateFmt(d, homeId, { weekday: 'short', month: 'long', day: 'numeric' }),
    'Today · W' + Calendar.isoWeek(y, m, day),
    '󰃭',
    () => {
      month = null;
    },
  );
  add('lastYear', '−1 yr', '', '', () => step(-12));
  add('lastMonth', '‹', '', '', () => step(-1));
  add(
    'month',
    dateFmt(new Date(v.year, v.month, 15), homeId, { month: 'long', year: 'numeric' }),
    '',
    '',
    null,
  );
  add('nextMonth', '›', '', '', () => step(1));
  add('nextYear', '+1 yr', '', '', () => step(12));
  for (const w of Calendar.monthGrid(v.year, v.month, start, Calendar.keyForDate(d))) {
    // A week per card, with weekday initials above their aligned dates.
    add('week/' + w.week, 'W' + w.week, '', '', null, {
      week: w.days.map((cell) => ({
        label: dateFmt(new Date(cell.year, cell.month, cell.day, 12), homeId, { weekday: 'narrow' }),
        day: String(cell.day),
        today: cell.today,
        dim: !cell.inMonth,
        weekend: cell.weekend,
      })),
    });
  }
  const next = Calendar.toggledWeekStart(start);
  add('weekStart', 'Week starts ' + Calendar.weekStartSettingName(start), 'Change first day', '', () =>
    persist(clock, { weekStartDay: Calendar.weekStartSettingName(next) }),
  );
  const yr = Calendar.yearProgress(y, m, day),
    born = Calendar.parseBirthYear(clock.birthYear, y);
  const span = Calendar.parseLifeExpectancy(clock.lifeExpectancy);
  add(
    'yearMeter',
    String(y) + ' · ' + Math.round(yr * 100) + '%',
    'Edit life progress',
    '',
    () => enter('birth', born ? String(born) : ''),
    { fraction: yr },
  );
  if (born) {
    const life = Calendar.lifeProgress(y - born, span);
    add(
      'lifeMeter',
      'Life · ' + Math.round(life * 100) + '%',
      'Clear life progress',
      '',
      () => persist(clock, { birthYear: 0 }),
      { fraction: life },
    );
  }
  const format = clock.format || 'dddd HH:mm';
  add('clockFormat', 'Clock format', format, '󰥔', () =>
    persist(clock, {
      format: Calendar.nextClockFormat(
        Calendar.clockFormatRing(
          format,
          clock.formatAlt || "d MMMM 'W'ww yyyy",
          Calendar.clockFormats(false),
        ),
        format,
      ),
    }),
  );
}
function editView() {
  const person = page === 'people',
    name = person ? selected()?.label : '';
  add(
    'field',
    query || (person ? 'Names or @handles, separated by commas' : 'Type a number'),
    person ? 'People in ' + name : page === 'birth' ? 'Birth year' : 'Life expectancy',
    '',
    null,
    { active: true },
  );
  add('save', page === 'birth' ? 'Next' : 'Save', '', '󰄬', submit);
  add('erase', '⌫', '', '', () => editText('backspace'));
  add('clear', 'Clear', '', '', () => editText('clear'));
}
async function submit() {
  if (page === 'people') {
    const key = focus,
      people = M.parsePeople(query);
    await editCities((z) => M.setPeopleAt(z, M.indexOfZoneKey(z, key), people));
    enter(selected() ? 'city' : 'list');
  } else if (page === 'birth') {
    birthInput = Calendar.parseBirthYear(query, new Date().getFullYear());
    enter('span', String(Calendar.parseLifeExpectancy(clock.lifeExpectancy)));
  } else if (page === 'span') {
    await persist(clock, { birthYear: birthInput, lifeExpectancy: Calendar.parseLifeExpectancy(query) });
    enter('calendar');
  } else {
    const choice = matches()[selection];
    if (choice) await pick(choice);
  }
}
function editText(op, text = '') {
  if (!editing()) return;
  selection = 0;
  if (replaceNumber) {
    query = '';
    replaceNumber = false;
  }
  if (op === 'clear') query = '';
  else if (op === 'backspace') query = Array.from(query).slice(0, -1).join('');
  else {
    const clean =
      page === 'birth' || page === 'span'
        ? text.replace(/[^0-9]/g, '')
        : text.replace(/[\u0000-\u001f\u007f]/g, '');
    query = Array.from(query + clean)
      .slice(0, page === 'people' ? 512 : 96)
      .join('');
  }
  if (['add', 'here', 'jump'].includes(page)) scheduleSearch();
}
function drag({ id, fraction, released }) {
  if (page === 'city' && id === 'daylight' && selected()) {
    const ms = Date.now(),
      p = M.zoneParts(ms, offset(selected().id, ms));
    shift = Math.round(M.scrubDeltaMinutes(fraction, p.hour * 60 + p.minute));
    clearTimeout(scrubReset);
    if (released)
      scrubReset = setTimeout(() => {
        shift = 0;
        if (session) render();
      }, 2500);
  } else if (page === 'globe' && id === 'earth') {
    spin = fraction * 360 - 180;
    if (released) {
      const rows = M.mergeCities(
        [],
        builtIn,
        cities.map((c) => [c.label, c.id, spot(c)?.lat, spot(c)?.lon]),
        [jumpCity].map((c) => [c.label, c.id, spot(c)?.lat, spot(c)?.lon]),
      );
      let nearest = null,
        distance = Infinity;
      for (const [label, id, lat, lon] of rows) {
        const p = Solar.project(lat, lon, spin, latitude, 1);
        const d = p.x * p.x + p.y * p.y;
        if (p.visible && d < distance) {
          distance = d;
          nearest = { label, id, lat, lon };
        }
      }
      if (nearest) jumpCity = nearest;
    }
  }
}
async function fetchJson(url) {
  const { stdout } = await run(
    'curl',
    ['-fsS', '--max-time', '6', '--max-filesize', '524288', '--proto', '=https', '--', url],
    { timeout: 8000, maxBuffer: 524288 },
  );
  return stdout;
}
let factsRunning = false,
  factsQueued = false;
async function refreshFacts(force = false) {
  if (!session || preview) return;
  if (factsRunning) {
    factsQueued = true;
    return;
  }
  factsRunning = true;
  try {
    const list = [...new Map([here, ...cities].map((c) => [cityKey(c), c])).values()];
    // Show the row immediately. Replace approximate zone coordinates with
    // the named place and attach current weather when the network answers.
    const stale = list.filter((c) => force || Date.now() - (weather.get(cityKey(c))?.at || 0) > 1200000);
    for (let i = 0; i < stale.length && session; i += 6) {
      const batch = stale.slice(i, i + 6);
      await Promise.all(
        batch.map(async (c) => {
          const k = cityKey(c);
          if (
            c.lat !== undefined ||
            geo.has(k) ||
            builtIn.some((p) => p[0] === c.label && p[1] === c.id)
          )
            return;
          try {
            const p = M.pickGeocode(await fetchJson(M.geocodeUrl(c.label)), c.id).place;
            if (p && M.isCoordinate(p.lat, 90) && M.isCoordinate(p.lon, 180)) geo.set(k, p);
          } catch {}
        }),
      );
      const known = batch.filter((c) => spot(c)),
        pts = known.map(spot),
        keys = known.map(cityKey);
      if (!known.length) continue;
      try {
        const result = M.parseForecast(await fetchJson(M.forecastUrl(pts)), keys, Date.now(), pts);
        for (const [k, v] of Object.entries(result)) weather.set(k, v);
      } catch {}
    }
  } finally {
    factsRunning = false;
    if (session) render();
    if (factsQueued) {
      factsQueued = false;
      refreshFacts();
    }
  }
}
async function fetchProfiles() {
  if (!session || preview) return;
  const wanted = M.handlesIn(cities).filter((h) => !triedProfiles.has(h));
  for (const h of wanted) triedProfiles.add(h);
  for (const h of wanted) {
    try {
      const [cmd, ...args] = M.profilesCommand([h]);
      const { stdout } = await run(cmd, args, { timeout: 50000 });
      const p = M.parseProfiles(stdout)[h];
      if (!p) continue;
      if (p.avatar) {
        // Hand the renderer a tiny fixed-size raster irrespective of the
        // format of GitHub's cached avatar. Names work without ImageMagick.
        try {
          const { stdout: pixels } = await run(
            'magick',
            [
              p.avatar + '[0]',
              '-thumbnail',
              '24x24^',
              '-gravity',
              'center',
              '-extent',
              '24x24',
              '-depth',
              '8',
              'rgba:-',
            ],
            { encoding: 'buffer', timeout: 3000 },
          );
          p.avatar = pixels.length === 24 * 24 * 4 ? pixels.toString('hex') : '';
        } catch {
          p.avatar = '';
        }
      }
      profiles[h] = p;
      if (session) render();
    } catch {}
  }
}
function render() {
  if (!session) return;
  actions = new Map();
  items = [];
  if (notice)
    add('notice', notice, 'Tap to dismiss', '󰀦', () => {
      notice = '';
    });
  if (page === 'city' && !selected()) enter('list');
  if (page === 'people' && !selected()) enter('list');
  switch (page) {
    case 'list':
      renderList();
      break;
    case 'city':
      renderCity(selected());
      break;
    case 'choosePeople':
      for (const c of cities)
        add('who/' + cityKey(c), c.label, M.peopleText(c.people) || 'Add people', '󰀎', () => {
          focus = cityKey(c);
          enter('people', M.peopleText(c.people));
        });
      break;
    case 'globe':
      renderGlobe();
      break;
    case 'calendar':
      calendarView();
      break;
    case 'add':
    case 'here':
    case 'jump':
      searchView();
      break;
    case 'people':
    case 'birth':
    case 'span':
      editView();
      break;
    case 'confirm':
      add(
        'applyHere',
        'Use ' + confirmHere.label,
        'Set system time zone to ' + confirmHere.id,
        '⌖',
        () => setHere(confirmHere),
      );
      break;
  }
  const nav = page === 'list' ? 'Time' : editing() || page === 'confirm' ? 'Cancel' : 'Back';
  process.stdout.write(JSON.stringify({ session, view: page, nav, editing: editing(), items }) + '\n');
}
function armTick() {
  clearTimeout(tick);
  tick = setTimeout(
    () => {
      if (!session) return;
      render();
      refreshFacts();
      armTick();
    },
    60020 - (Date.now() % 60000),
  );
}
async function dispatch(req) {
  if (req.op === 'open') {
    clearTimeout(retire);
    clearTimeout(scrubReset);
    session = req.session;
    focus = '';
    shift = 0;
    enter('list');
    loadModels();
    readSettings();
    armTick();
    refreshFacts();
    fetchProfiles();
  } else {
    if (!session || req.session !== session) return;
    switch (req.op) {
      case 'close':
        session = 0;
        if (!preview) retire = setTimeout(() => process.exit(0), 60000);
        clearTimeout(tick);
        clearTimeout(debounce);
        clearTimeout(scrubReset);
        return;
      case 'reload':
        readSettings();
        refreshFacts();
        fetchProfiles();
        break;
      case 'back':
        back();
        break;
      case 'tap':
        await actions.get(req.value)?.();
        fetchProfiles();
        break;
      case 'cancelDrag':
        clearTimeout(scrubReset);
        shift = 0;
        break;
      case 'drag':
        drag(req.value);
        break;
      case 'select':
        if (['add', 'jump', 'here'].includes(page))
          selection = M.moveSelection(selection, req.value, matches().length);
        break;
      case 'enter':
        if (editing()) await submit();
        fetchProfiles();
        break;
      case 'text':
      case 'backspace':
      case 'clear':
        editText(req.op, req.value);
        break;
    }
  }
  render();
}
(async () => {
  for await (const line of readline.createInterface({ input: process.stdin })) {
    try {
      await dispatch(JSON.parse(line));
    } catch (e) {
      const message = String(e.message || e)
        .split('\n')[0]
        .slice(0, 180);
      if (session && cities) {
        notice = message;
        try {
          render();
          continue;
        } catch {}
      }
      process.stdout.write(JSON.stringify({ session, error: message }) + '\n');
    }
  }
  process.exit(0);
})();
