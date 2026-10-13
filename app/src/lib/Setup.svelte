<script>
  import { untrack } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { open } from '@tauri-apps/plugin-dialog';
  import { enable } from '@tauri-apps/plugin-autostart';
  import tile from './tile.svg';
  import { size } from './format.js';

  let { info } = $props();

  // 1 welcome, 2 code, 3 folder, 4 projects
  let step = $state(untrack(() => (info.status.state === 'setup' ? 3 : 1)));
  let code = $state(null);
  let error = $state('');
  let folder = $state(untrack(() => info.folder));
  let projects = $state(null);
  let chosen = $state({});

  $effect(() => {
    if (info.status.state === 'setup' && step < 3) step = 3;
  });

  $effect(() => {
    const off = listen('login-failed', (e) => {
      error = e.payload;
      step = 1;
    });
    return () => off.then((f) => f());
  });

  async function login() {
    error = '';
    try {
      code = await invoke('login_start');
      step = 2;
    } catch (e) {
      error = String(e);
    }
  }

  async function pickFolder() {
    const picked = await open({ directory: true, defaultPath: folder });
    if (picked) folder = picked;
  }

  async function toProjects() {
    step = 4;
    try {
      projects = await invoke('projects');
      // Big archives start unticked, everything else syncs
      chosen = Object.fromEntries(projects.map((p) => [p.id, p.bytes < 10 * 1024 ** 3]));
    } catch (e) {
      error = String(e);
    }
  }

  const total = $derived(projects ? projects.filter((p) => chosen[p.id]).reduce((sum, p) => sum + p.bytes, 0) : 0);

  async function finish() {
    const excluded = projects.filter((p) => !chosen[p.id]).map((p) => p.id);
    await enable().catch(() => {});
    await invoke('configure', { folder, excluded, paused: false });
  }
</script>

<div class="setup">
  <div class="steps">{#each [1, 2, 3, 4] as i}<i class:on={i <= step}></i>{/each}</div>

  {#if step === 1}
    <img src={tile} width="52" alt="" />
    <h1 class="display">Dein Studio-Ordner, verbunden mit Music Hub</h1>
    <p class="muted">Bounces, die du hier speicherst, landen als neue Version im Hub. Was deine Band hochlädt, liegt automatisch hier.</p>
    {#if error}<p class="error">{error}</p>{/if}
    <div class="row"><button class="btn primary" onclick={login}>Anmelden</button></div>
  {:else if step === 2}
    <h1 class="display">Bestätige im Browser</h1>
    <p class="muted">Der Browser hat sich geöffnet. Prüf, dass dort derselbe Code steht, und tipp auf „Verbinden“.</p>
    <div class="code">{code.userCode}</div>
    <div class="wait"><div class="spin"></div> Warte auf Bestätigung …</div>
    <div class="row">
      <button class="btn" onclick={() => invoke('open_verification', { url: code.verificationUrl })}>Browser erneut öffnen</button>
    </div>
  {:else if step === 3}
    <h1 class="display">Wo soll dein Ordner liegen?</h1>
    <p class="muted">Leg ihn dorthin, wo deine DAW sowieso exportiert. Später änderbar.</p>
    <div class="field">
      <span class="path" title={folder}>{folder}</span>
      <button class="btn" onclick={pickFolder}>Ändern</button>
    </div>
    <div class="row"><button class="btn primary" onclick={toProjects}>Weiter</button></div>
  {:else}
    <h1 class="display">Welche Projekte?</h1>
    <p class="muted">Ausgewählte Projekte liegen komplett lokal, mit allen Versionen und Spuren.</p>
    {#if error}<p class="error">{error}</p>{/if}
    {#if !projects}
      <div class="wait"><div class="spin"></div> Lade Projekte …</div>
    {:else}
      <div class="checks">
        {#each projects as p (p.id)}
          <label>
            <input type="checkbox" bind:checked={chosen[p.id]} />
            <span class="name">{p.name}</span>
            <span class="muted sz">{size(p.bytes)} · {p.tracks} Tracks</span>
          </label>
        {:else}
          <p class="muted">Noch keine Projekte. Leg im Ordner einen an, er wird zum Projekt.</p>
        {/each}
      </div>
      <div class="row">
        <button class="btn primary" onclick={finish}>{total > 0 ? `${size(total)} laden und loslegen` : 'Loslegen'}</button>
      </div>
    {/if}
  {/if}
</div>

<style>
  .setup {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 22px 24px;
    min-height: 0;
  }
  h1 {
    font-size: 26px;
    line-height: 1.1;
    margin: 4px 0 0;
  }
  p {
    margin: 0;
  }
  .steps {
    display: flex;
    gap: 6px;
  }
  .steps i {
    height: 3px;
    flex: 1;
    background: var(--overlay);
    border-radius: 3px;
  }
  .steps i.on {
    background: var(--grad);
  }
  .row {
    margin-top: auto;
    display: flex;
    gap: 8px;
  }
  .row .primary {
    flex: 1;
  }
  .code {
    font: 700 34px/1 ui-monospace, 'SF Mono', Menlo, monospace;
    letter-spacing: 0.1em;
    padding: 14px 16px;
    background: var(--raised);
    border-radius: 12px;
    align-self: flex-start;
    user-select: text;
  }
  .wait {
    display: flex;
    gap: 8px;
    align-items: center;
    color: var(--muted);
  }
  .field {
    display: flex;
    gap: 10px;
    align-items: center;
    padding: 8px 8px 8px 12px;
    background: var(--raised);
    border-radius: 10px;
  }
  .path {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: rtl;
    text-align: left;
  }
  .checks {
    display: flex;
    flex-direction: column;
    gap: 4px;
    overflow: auto;
    min-height: 0;
  }
  label {
    display: flex;
    gap: 10px;
    align-items: center;
    padding: 8px 10px;
    border-radius: 9px;
    background: var(--raised);
  }
  label input {
    accent-color: var(--accent);
  }
  .name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sz {
    font-size: 12px;
    white-space: nowrap;
  }
  .error {
    color: var(--error);
  }
</style>
