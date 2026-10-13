<script>
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import tile from './tile.svg';
  import { ago, agoSecs, size } from './format.js';

  let { info, onsettings } = $props();

  let events = $state([]);
  let feedError = $state('');

  async function loadFeed() {
    try {
      events = (await invoke('activity')).events ?? [];
      feedError = '';
    } catch (e) {
      feedError = String(e);
    }
  }

  onMount(() => {
    loadFeed();
    const off = listen('opened', loadFeed);
    return () => off.then((f) => f());
  });

  const status = $derived(info.status);
  const synced = $derived(status.projects.filter((p) => !info.excluded.includes(p.id)));
  const bytes = $derived(synced.reduce((sum, p) => sum + p.bytes, 0));

  function who(e) {
    return e.user?.name ?? e.guestName ?? 'Jemand';
  }
  function what(e) {
    const v = e.version ? `V${e.version.versionNumber}` : '';
    if (e.type === 'version') return `hat <b>${v}</b> von <b>${escape(e.track.name)}</b> hochgeladen`;
    if (e.type === 'approval') return `hat <b>${v}</b> von <b>${escape(e.track.name)}</b> ${e.status === 'rejected' ? 'abgelehnt' : 'freigegeben'}`;
    return `zu <b>${v}</b> · <b>${escape(e.track.name)}</b>: „${escape((e.body ?? '').slice(0, 80))}“`;
  }
  function escape(s) {
    return s.replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[c]);
  }
  function initials(name) {
    return name.split(/\s+/).map((w) => w[0]).join('').slice(0, 2).toUpperCase();
  }
  const COLORS = ['#46BFE0', '#6CCF8E', '#C39BF2', '#F99747', '#8E92F8', '#F2C14E'];
  function color(name) {
    let h = 0;
    for (const c of name) h = (h * 31 + c.charCodeAt(0)) >>> 0;
    return COLORS[h % COLORS.length];
  }
  const action = { version: 'Anhören', approval: 'Ansehen', comment: 'Lesen' };
</script>

<header>
  <img src={tile} width="30" alt="" />
  <div>
    <div class="display title">Music Hub</div>
    <div class="muted small">{info.email ?? ''}</div>
  </div>
  <span class="grow"></span>
  <button class="icon" title="Einstellungen" aria-label="Einstellungen" onclick={onsettings}>
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="3" /><path d="M12 2v3M12 19v3M4.2 4.2l2.1 2.1M17.7 17.7l2.1 2.1M2 12h3M19 12h3M4.2 19.8l2.1-2.1M17.7 6.3l2.1-2.1" /></svg>
  </button>
</header>

<section class="status">
  {#if status.state === 'syncing'}
    <div class="spin"></div>
    <div><b>Gleicht ab …</b><div class="muted small">{status.busy ? `${status.busy} Datei${status.busy === 1 ? '' : 'en'} wird noch geschrieben` : 'Lädt hoch und herunter'}</div></div>
  {:else if status.state === 'paused'}
    <span class="pill" style="background: var(--muted)"></span>
    <div><b>Pausiert</b><div class="muted small">Weiter geht es in den Einstellungen</div></div>
  {:else if status.state === 'offline' || status.state === 'error'}
    <span class="pill" style="background: var(--error)"></span>
    <div><b>{status.state === 'offline' ? 'Keine Verbindung' : 'Fehler'}</b><div class="muted small">{status.message ?? ''}</div></div>
  {:else}
    <span class="pill" style="background: var(--new)"></span>
    <div>
      <b>Alles synchron</b>
      <div class="muted small">{synced.length} Projekt{synced.length === 1 ? '' : 'e'} · {size(bytes)}{status.lastSync ? ` · geprüft ${agoSecs(status.lastSync)}` : ''}</div>
    </div>
  {/if}
</section>

{#if status.problems.length}
  <div class="sec">Braucht dich</div>
  <ul class="feed">
    {#each status.problems as p}
      <li>
        <span class="av" style="background: var(--error)">!</span>
        <div class="what"><b>{p.path.split('/').pop()}</b><div class="muted small">{p.message}</div></div>
        <button class="act" onclick={() => invoke('open_folder', { path: p.path.split('/').slice(0, -1).join('/') })}>Zeigen</button>
      </li>
    {/each}
  </ul>
{/if}

<div class="sec">Neu in deinen Projekten</div>
<ul class="feed grow-list">
  {#each events as e (e.type + e.id)}
    <li>
      <span class="av" style="background: {color(who(e))}">{initials(who(e))}</span>
      <div class="what">
        <b>{who(e)}</b> {@html what(e)}
        <div class="muted small">{e.project.name} · {ago(e.createdAt)}</div>
      </div>
      <button class="act" onclick={() => invoke('open_hub', { path: `/projects/${e.project.id}/tracks/${e.track.id}` })}>{action[e.type]} ↗</button>
    </li>
  {:else}
    <li class="muted small">{feedError || 'Noch nichts Neues.'}</li>
  {/each}
</ul>

<footer>
  <button class="btn" onclick={() => invoke('open_folder', {})}>Ordner öffnen</button>
  <button class="btn primary" onclick={() => invoke('open_hub', {})}>Music Hub öffnen ↗</button>
</footer>

<style>
  header {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 14px 14px 12px;
  }
  .title {
    font-size: 17px;
  }
  .small {
    font-size: 12px;
  }
  .grow {
    flex: 1;
  }
  .icon {
    border: 0;
    background: none;
    color: var(--muted);
    padding: 6px;
    border-radius: 7px;
  }
  .icon:hover {
    background: var(--overlay);
    color: var(--text);
  }
  .status {
    margin: 0 14px 10px;
    padding: 10px 12px;
    border-radius: 10px;
    background: var(--raised);
    display: flex;
    gap: 10px;
    align-items: center;
  }
  .pill {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    flex: none;
  }
  .sec {
    padding: 6px 14px 2px;
    color: var(--muted);
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }
  .feed {
    list-style: none;
    margin: 0;
    padding: 4px 6px 6px;
  }
  .grow-list {
    flex: 1;
    overflow: auto;
    min-height: 0;
  }
  .feed li {
    display: flex;
    gap: 10px;
    padding: 8px;
    border-radius: 9px;
    align-items: flex-start;
  }
  .feed li:hover {
    background: var(--raised);
  }
  .av {
    width: 28px;
    height: 28px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    font-size: 11px;
    font-weight: 700;
    color: var(--on-grad);
    flex: none;
  }
  .what {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .what :global(b) {
    font-weight: 600;
  }
  .act {
    border: 1px solid var(--line);
    background: none;
    border-radius: 8px;
    padding: 4px 9px;
    font-size: 12px;
    white-space: nowrap;
  }
  .act:hover {
    border-color: var(--accent);
  }
  footer {
    display: flex;
    gap: 8px;
    padding: 10px 14px 14px;
    border-top: 1px solid var(--line);
  }
  footer .btn {
    flex: 1;
  }
</style>
