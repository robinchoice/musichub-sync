<script>
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import Setup from './lib/Setup.svelte';
  import Popover from './lib/Popover.svelte';
  import Settings from './lib/Settings.svelte';

  let info = $state(null);
  let settings = $state(false);

  async function refresh() {
    info = await invoke('status');
  }

  onMount(() => {
    refresh();
    const off = listen('status', refresh);
    return () => off.then((f) => f());
  });

  const setup = $derived(info && ['login', 'setup'].includes(info.status.state || 'login'));
</script>

<main>
  {#if !info}
    <div class="center"><div class="spin"></div></div>
  {:else if setup || !info.status.state}
    <Setup {info} />
  {:else if settings}
    <Settings {info} onback={() => (settings = false)} />
  {:else}
    <Popover {info} onsettings={() => (settings = true)} />
  {/if}
</main>

<style>
  main {
    height: 100vh;
    display: flex;
    flex-direction: column;
    border: 1px solid var(--line);
  }
  .center {
    flex: 1;
    display: grid;
    place-items: center;
  }
</style>
