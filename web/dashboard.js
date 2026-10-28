'use strict';
const $ = (id) => document.getElementById(id);
let csrf = '', current = '', baseline = '', saving = false, sequence = 0;
let serverList = [];

function notice(message, kind = 'info') {
  $('notice').textContent = message;
  $('notice').dataset.kind = kind;
  $('notice').hidden = !message;
}

async function api(path, options = {}) {
  const response = await fetch(`/music${path}`, { credentials: 'same-origin', ...options,
    headers: { ...options.headers, ...(options.method ? { 'X-CSRF-Token': csrf } : {}) } });
  if (response.status === 204) return null;
  let data;
  try { data = await response.json(); } catch { throw new Error('The server could not respond. Please try again.'); }
  if (!response.ok) {
    if (response.status === 401) signedOut();
    throw new Error(data.error || 'The request failed. Please try again.');
  }
  return data;
}

function signedOut() {
  csrf = ''; current = ''; baseline = ''; sequence++;
  for (const id of ['editor', 'sidebar', 'loading', 'empty']) $(id).hidden = true;
  $('welcome').hidden = false; renderAccount(null);
}

function policy() {
  const form = $('settings-form');
  return { access: form.elements.access.value, mode: form.elements.mode.value,
    majority_scope: $('majority-scope').value,
    moderator_roles: [...$('roles').querySelectorAll('input:checked')].map((input) => input.value).sort() };
}

function changed() { return !!baseline && JSON.stringify(policy()) !== baseline; }

function update() {
  const selected = policy();
  $('role-options').hidden = selected.access !== 'moderators';
  $('majority-options').hidden = selected.mode !== 'majority';
  $('majority-help').textContent = selected.majority_scope === 'eligible'
    ? 'More than half of eligible listeners must vote. With 2 eligible moderators, both must agree.'
    : selected.access === 'moderators'
      ? 'All human listeners count, but only moderators can vote. With 10 listeners, 6 votes are needed. If fewer than 6 are eligible, the vote cannot pass.'
      : 'More than half of all human listeners must vote. With 5 listeners, 3 votes are needed.';
  const dirty = changed();
  $('save').disabled = saving || !dirty;
  $('save-state').textContent = saving ? 'Saving your changes…' : dirty ? 'You have unsaved changes' : 'All changes saved';
  $('save-state').dataset.state = dirty ? 'unsaved' : 'saved';
  $('preview-access').textContent = selected.access === 'moderators' ? 'Moderators only' : 'Everyone';
  $('preview-mode').textContent = selected.mode === 'majority' ? 'Majority vote' : 'One click';
  $('preview-title').textContent = selected.mode === 'majority' ? 'Let the room decide.' : 'One click. Next track.';
  $('preview-description').textContent = selected.mode === 'instant'
    ? selected.access === 'moderators'
      ? 'An eligible moderator in the call can skip a track immediately.'
      : 'Any listener in the call can skip a track immediately.'
    : selected.access === 'moderators'
      ? selected.majority_scope === 'eligible'
        ? 'Only eligible moderators vote. More than half of them must agree.'
        : 'Only eligible moderators vote. More than half of all listeners must agree.'
      : 'Listeners in the call vote together. More than half must agree.';
}

function renderServers() {
  $('servers').replaceChildren();
  for (const guild of serverList) {
    const button = document.createElement('button');
    button.className = 'server'; button.type = 'button';
    button.setAttribute('aria-current', String(guild.id === current));
    const initial = document.createElement('span');
    initial.className = 'server-initial'; initial.textContent = guild.name.slice(0, 1).toUpperCase();
    initial.setAttribute('aria-hidden', 'true');
    const name = document.createElement('span'); name.textContent = guild.name;
    button.append(initial, name);
    button.addEventListener('click', () => {
      if (guild.id === current || saving) return;
      if (changed() && !window.confirm('Discard your unsaved changes and switch servers?')) return;
      loadServer(guild.id);
    });
    $('servers').append(button);
  }
}

async function loadServer(id) {
  const request = ++sequence;
  current = id; baseline = ''; notice(''); renderServers();
  $('editor').hidden = true; $('empty').hidden = true; $('loading').hidden = false;
  $('loading').querySelector('p').textContent = 'Loading server settings.';
  try {
    const data = await api(`/api/guilds/${encodeURIComponent(id)}/settings`);
    if (request !== sequence) return;
    $('server-name').textContent = data.name;
    const form = $('settings-form');
    form.elements.access.value = data.settings.access;
    form.elements.mode.value = data.settings.mode;
    $('majority-scope').value = data.settings.majority_scope;
    $('roles').replaceChildren();
    const available = new Set(data.roles.map((role) => role.id));
    for (const role of data.roles) {
      const label = document.createElement('label'); label.className = 'role';
      const input = document.createElement('input'); input.type = 'checkbox'; input.value = role.id;
      input.checked = data.settings.moderator_roles.includes(role.id);
      const text = document.createElement('span'); text.textContent = role.name;
      label.append(input, text); $('roles').append(label);
    }
    baseline = JSON.stringify(policy());
    $('editor').hidden = false;
    history.replaceState(null, '', `/music/dashboard?server=${encodeURIComponent(id)}`);
    update();
    if (data.settings.moderator_roles.some((role) => !available.has(role))) {
      notice('A previously selected role was deleted. Review the roles and save to remove it from your settings.');
      baseline = 'deleted-roles'; update();
    }
  } catch (error) {
    if (request === sequence) { notice(error.message, 'error'); $('empty').hidden = !csrf; }
  } finally { if (request === sequence) $('loading').hidden = true; }
}

async function refreshServers() {
  $('loading').hidden = false; $('empty').hidden = true; notice('');
  try {
    const data = await api('/api/guilds');
    serverList = data.guilds; renderServers(); $('loading').hidden = true;
    if (!serverList.length) { $('editor').hidden = true; $('empty').hidden = false; return; }
    const wanted = new URLSearchParams(location.search).get('server');
    await loadServer(serverList.find((guild) => guild.id === wanted)?.id || serverList[0].id);
  } catch (error) {
    $('loading').hidden = true; $('empty').hidden = !csrf; notice(error.message, 'error');
  }
}

$('settings-form').addEventListener('change', update);
$('settings-form').addEventListener('submit', async (event) => {
  event.preventDefault();
  if (saving || !current || !changed()) return;
  const selected = policy();
  if (selected.moderator_roles.length > 50) { notice('Choose at most 50 moderator roles.', 'error'); return; }
  saving = true; $('save').textContent = 'Saving…'; update(); notice('');
  const controls = [...$('settings-form').querySelectorAll('input, select')];
  controls.forEach((control) => { control.disabled = true; });
  try {
    await api(`/api/guilds/${encodeURIComponent(current)}/settings`, {
      method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(selected)
    });
    baseline = JSON.stringify(selected);
    notice('Settings saved. Current skip votes have been cleared.', 'success');
  } catch (error) { notice(error.message, 'error'); }
  finally {
    saving = false; controls.forEach((control) => { control.disabled = false; });
    $('save').textContent = 'Save changes'; update();
  }
});
$('logout').addEventListener('click', async () => {
  if (saving || (changed() && !window.confirm('Discard your unsaved changes and sign out?'))) return;
  $('logout').disabled = true;
  try { await api('/auth/logout', { method: 'POST' }); signedOut(); notice(''); }
  catch (error) { notice(error.message, 'error'); }
  finally { $('logout').disabled = false; }
});
$('refresh').addEventListener('click', refreshServers);
window.addEventListener('beforeunload', (event) => { if (changed() || saving) { event.preventDefault(); event.returnValue = ''; } });

(async () => {
  try {
    const data = await api('/api/me'); csrf = data.csrf;
    renderAccount(data.user);
    $('sidebar').hidden = false;
    await refreshServers();
  } catch (error) {
    signedOut();
    if (location.search.includes('login=cancelled')) notice('Discord sign-in was cancelled. You can try again.');
    else if (!error.message.includes('Sign in') && !error.message.includes('session expired')) notice(error.message, 'error');
  }
})();

function renderAccount(user) {
  const link = $('navLogin');
  link.replaceChildren();
  link.classList.toggle('nav__login--account', !!user);
  if (!user) { link.textContent = 'Log in'; link.removeAttribute('aria-label'); return; }
  const name = user.global_name || user.username;
  link.setAttribute('aria-label', `Discord account: ${name}`);
  const avatar = document.createElement('span');
  avatar.className = 'discord-avatar nav__login-av-wrap';
  const image = document.createElement('img');
  image.className = 'nav__login-av'; image.width = 24; image.height = 24;
  image.alt = `${name} avatar`; image.referrerPolicy = 'no-referrer';
  const initial = document.createElement('span');
  initial.className = 'nav__login-av nav__login-av--none';
  initial.textContent = name.slice(0, 1).toUpperCase();
  if (/^\d{16,22}$/.test(user.id) && /^(?:a_)?[a-f0-9]{32}$/.test(user.avatar || '')) {
    image.src = `https://cdn.discordapp.com/avatars/${user.id}/${user.avatar}.webp?size=32`;
    image.addEventListener('error', () => image.replaceWith(initial), {once:true});
    avatar.append(image);
  } else avatar.append(initial);
  const asset = user.avatar_decoration_data?.asset;
  if (/^[A-Za-z0-9_]{1,128}$/.test(asset || '')) {
    const decoration = document.createElement('img');
    decoration.className = 'discord-avatar__decoration'; decoration.alt = '';
    decoration.setAttribute('aria-hidden', 'true'); decoration.width = 256; decoration.height = 256;
    decoration.referrerPolicy = 'no-referrer';
    decoration.src = `https://cdn.discordapp.com/avatar-decoration-presets/${asset}.png?size=256`;
    decoration.addEventListener('error', () => decoration.remove(), {once:true});
    avatar.append(decoration);
  }
  const label = document.createElement('span'); label.textContent = name;
  link.append(avatar, label);
}

function closeNavigation() {
  $('vozen-primary-links').classList.remove('is-open');
  $('burger').setAttribute('aria-expanded', 'false');
  $('langMenu').classList.remove('is-open');
  $('langBtn').setAttribute('aria-expanded', 'false');
}
$('burger').addEventListener('click', () => {
  const open = $('burger').getAttribute('aria-expanded') !== 'true';
  closeNavigation();
  $('vozen-primary-links').classList.toggle('is-open', open);
  $('burger').setAttribute('aria-expanded', String(open));
});
$('langBtn').addEventListener('click', () => {
  const open = $('langBtn').getAttribute('aria-expanded') !== 'true';
  closeNavigation();
  $('langMenu').classList.toggle('is-open', open);
  $('langBtn').setAttribute('aria-expanded', String(open));
  if (open) $('langPanel').querySelector('a')?.focus();
});
$('langPanel').addEventListener('keydown', event => {
  const options = [...$('langPanel').querySelectorAll('a')];
  const index = options.indexOf(document.activeElement);
  let next;
  if (event.key === 'ArrowDown') next = (index + 1) % options.length;
  else if (event.key === 'ArrowUp') next = (index + options.length - 1) % options.length;
  else if (event.key === 'Home') next = 0;
  else if (event.key === 'End') next = options.length - 1;
  if (next !== undefined) { event.preventDefault(); options[next].focus(); }
});
document.addEventListener('keydown', event => {
  if (event.key !== 'Escape' || ($('langBtn').getAttribute('aria-expanded') !== 'true' && $('burger').getAttribute('aria-expanded') !== 'true')) return;
  const button = $('langBtn').getAttribute('aria-expanded') === 'true' ? $('langBtn') : $('burger');
  closeNavigation(); button.focus();
});
document.addEventListener('click', event => { if (!$('nav').contains(event.target)) closeNavigation(); });
window.matchMedia('(min-width:1280px)').addEventListener('change', event => { if (event.matches) closeNavigation(); });
window.addEventListener('scroll', () => $('nav').classList.toggle('is-stuck', scrollY > 12), {passive:true});
