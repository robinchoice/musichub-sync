<script>
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { open } from '@tauri-apps/plugin-dialog';
  import { enable, disable, isEnabled } from '@tauri-apps/plugin-autostart';
  import { size } from './format.js';

  let { info, onback } = $props();

  let autostart = $state(false);
  let choosing = $state(false);

  onMount(async () => {
    autostart = await isEnabled().catch(() => false);
  });

  async function pickFolder() {
    const folder = await open({ directory: true, defaultPath: info.folder });
    if (folder) await invoke('configure', { folder });
  }

  async function toggleAutostart() {
    autostart ? await disable() : await enable();
    autostart = await isEnabled();
  }
</script>

<div class="top">
  <button class="back" onclick={onback}>← Zurück</button>
</div>

<div class="settings">
  <div class="group">
    <div class="it">
      <div class="grow"><div>Ordner</div><div class="muted small path" title={info.folder}>{info.folder}</div></div>
      <button class="btn" onclick={pickFolder}>Ändern</button>
    </div>
    <div class="it">
      <div class="grow">
        <div>Projekte</div>
        <div class="muted small">{info.status.projects.length - info.excluded.length} von {info.status.projects.length} auf diesem Rechner</div>
      </div>
      <button class="btn" onclick={() => (choosing = !choosing)}>{choosing ? 'Fertig' : 'Auswählen'}</button>
    </div>
    {#if choosing}
      {#each info.status.projects as p (p.id)}
        <label class="it">
          <input type="checkbox" checked={!info.excluded.includes(p.id)} onchange={(e) => invoke('sync_project', { id: p.id, sync: e.currentTarget.checked })} />
          <span class="grow">{p.name}</span>
          <span class="muted small">{size(p.bytes)}</span>
        </label>
      {/each}
      <div class="it muted small">Abgewählte Projekte bleiben, wie sie sind, und werden nicht mehr abgeglichen.</div>
    {/if}
  </div>

  <div class="group">
    <div class="it">
      <span class="grow">Beim Anmelden starten</span>
      <button class="sw" class:on={autostart} aria-label="Beim Anmelden starten" onclick={toggleAutostart}></button>
    </div>
    <div class="it">
      <span class="grow">Abgleich pausieren</span>
      <button class="sw" class:on={info.paused} aria-label="Abgleich pausieren" onclick={() => invoke('configure', { paused: !info.paused })}></button>
    </div>
    <div class="it">
      <div class="grow"><div>Mails zu neuen Versionen</div><div class="muted small">Einstellung im Hub, gilt überall</div></div>
      <button class="btn" onclick={() => invoke('open_hub', { path: '/account' })}>Im Hub ↗</button>
    </div>
  </div>

  <div class="group">
    <div class="it">
      <div class="grow"><div>{info.email ?? ''}</div><div class="muted small">Music Hub Sync {info.version}</div></div>
      <button class="btn" onclick={() => invoke('logout')}>Abmelden</button>
    </div>
    <div class="it">
      <span class="grow muted small">Die App läuft im Hintergrund weiter, bis du sie beendest.</span>
      <button class="btn" onclick={() => invoke('quit')}>Beenden</button>
    </div>
  </div>
</div>

<style>
  .top {
    padding: 12px 14px 4px;
  }
  .back {
    border: 0;
    background: none;
    color: var(--muted);
    padding: 4px 6px;
    border-radius: 7px;
  }
  .back:hover {
    color: var(--text);
    background: var(--overlay);
  }
  .settings {
    flex: 1;
    overflow: auto;
    padding: 4px 14px 14px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .group {
    background: var(--raised);
    border-radius: 12px;
    padding: 2px 0;
  }
  .it {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 9px 12px;
  }
  .it + .it {
    border-top: 1px solid var(--line);
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .small {
    font-size: 12px;
  }
  .path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  label input {
    accent-color: var(--accent);
  }
  .sw {
    width: 34px;
    height: 20px;
    border-radius: 20px;
    background: var(--overlay);
    position: relative;
    border: 0;
    flex: none;
  }
  .sw::after {
    content: '';
    position: absolute;
    left: 3px;
    top: 3px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: #fff;
    transition: left 0.15s;
  }
  .sw.on {
    background: var(--grad);
  }
  .sw.on::after {
    left: 17px;
  }
</style>
