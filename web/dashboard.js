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
  for (const id of ['editor', 'sidebar', 'account', 'loading', 'empty']) $(id).hidden = true;
  $('welcome').hidden = false;
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
  $('save').disabled = saving || !changed();
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
    const data = await api(`/api/guilds/${encodeURIComponent(current)}/settings`, {
      method: 'PUT', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(selected)
    });
    data.settings.moderator_roles.sort(); baseline = JSON.stringify(data.settings);
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
    $('username').textContent = data.user.username;
    $('account').hidden = false; $('sidebar').hidden = false;
    await refreshServers();
  } catch (error) {
    signedOut();
    if (location.search.includes('login=cancelled')) notice('Discord sign-in was cancelled. You can try again.');
    else if (!error.message.includes('Sign in') && !error.message.includes('session expired')) notice(error.message, 'error');
  }
})();
