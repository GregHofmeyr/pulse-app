<script lang="ts">
  import { onMount } from 'svelte'
  import { getCurrentWindow } from '@tauri-apps/api/window'
  import { api, errorText } from '../lib/tauri'
  import type { User } from '../lib/protocol/User'
  import type { Server } from '../lib/protocol/Server'

  let { user, onLogout }: { user: User; onLogout: () => void } = $props()

  let servers = $state<Server[]>([])
  let active = $state<string>('home')
  let error = $state('')
  const win = getCurrentWindow()

  onMount(async () => {
    try {
      servers = await api.listServers()
    } catch (e) {
      error = errorText(e)
    }
  })

  const activeServer = $derived(servers.find((s) => s.id === active))
  const initial = (name: string) => name.trim().charAt(0).toUpperCase()

  async function logout() {
    await api.logout()
    onLogout()
  }
</script>

<div class="app">
  <header data-tauri-drag-region>
    <span class="logo" aria-hidden="true">
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round"><path d="M4 10v4M8 7v10M12 4v16M16 7v10M20 10v4" /></svg>
    </span>
    <nav aria-label="Servers">
      <button class="tab" class:active={active === 'home'} onclick={() => (active = 'home')}>
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 11 12 4l9 7" /><path d="M5 10v10h14V10" /></svg>
        Home
      </button>
      <span class="sep"></span>
      {#each servers as s (s.id)}
        <button class="tab" class:active={active === s.id} onclick={() => (active = s.id)}>
          <span class="chip">{initial(s.name)}</span>{s.name}
        </button>
      {/each}
    </nav>
    <div class="grow" data-tauri-drag-region></div>
    <button class="win" aria-label="Minimise" onclick={() => win.minimize()}>
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M5 12h14" /></svg>
    </button>
    <button class="win" aria-label="Maximise" onclick={() => win.toggleMaximize()}>
      <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="4" y="4" width="16" height="16" rx="3" /></svg>
    </button>
    <button class="win" aria-label="Close" onclick={() => win.close()}>
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M6 6l12 12M18 6 6 18" /></svg>
    </button>
  </header>

  <div class="body">
    <aside class="panel side">
      <div class="panel-head">{activeServer ? activeServer.name : 'Direct messages'}</div>
      <div class="side-list"></div>
      <div class="me">
        <span class="avatar">{initial(user.username)}</span>
        <span class="who">{user.username}<small>Online</small></span>
        <button class="ghost" onclick={logout}>Log out</button>
      </div>
    </aside>
    <main class="panel content">
      {#if error}
        <p class="empty error">{error}</p>
      {:else}
        <p class="empty">{activeServer ? 'Channels arrive in the next milestone.' : 'Your DMs will live here.'}</p>
      {/if}
    </main>
    <aside class="panel members"></aside>
  </div>
</div>

<style>
  .app { height: 100%; display: flex; flex-direction: column; background: var(--bg-0); }
  header { height: 52px; flex-shrink: 0; display: flex; align-items: center; gap: 6px; padding: 0 10px 0 12px; }
  .logo { width: 30px; height: 30px; border-radius: 9px; background: var(--accent); color: var(--on-accent); display: grid; place-items: center; margin-right: 8px; }
  nav { display: flex; align-items: center; gap: 6px; min-width: 0; overflow: hidden; }
  .tab { height: 36px; padding: 0 12px 0 10px; border-radius: 10px; border: 1px solid transparent; background: transparent; color: var(--text-2); display: flex; align-items: center; gap: 8px; font-size: 13px; font-weight: 500; white-space: nowrap; }
  .tab:hover { background: var(--bg-2); }
  .tab.active { background: #262830; border-color: var(--bg-4); color: #f6f7f9; font-weight: 600; }
  .chip { width: 22px; height: 22px; border-radius: 7px; background: #4a5bd4; color: #fff; font-size: 11px; font-weight: 700; display: grid; place-items: center; }
  .sep { width: 1px; height: 20px; background: var(--line); margin: 0 4px; }
  .grow { flex: 1; align-self: stretch; }
  .win { width: 32px; height: 28px; border: 0; border-radius: 7px; background: transparent; color: var(--text-3); display: grid; place-items: center; }
  .win:hover { background: var(--bg-2); color: var(--text); }
  .body { flex: 1; min-height: 0; display: flex; gap: 8px; padding: 0 8px 8px; }
  .panel { border-radius: var(--radius); overflow: hidden; }
  .side { width: 264px; background: var(--bg-1); display: flex; flex-direction: column; }
  .panel-head { height: 52px; padding: 0 16px; display: flex; align-items: center; font-size: 15px; font-weight: 600; border-bottom: 1px solid #26282e; }
  .side-list { flex: 1; }
  .me { padding: 10px 10px 10px 12px; background: #17181c; display: flex; align-items: center; gap: 10px; }
  .avatar { width: 34px; height: 34px; border-radius: 50%; background: #4a5bd4; color: #fff; font-weight: 700; display: grid; place-items: center; }
  .who { flex: 1; display: flex; flex-direction: column; font-size: 13px; font-weight: 600; }
  .who small { font-size: 12px; font-weight: 400; color: var(--text-3); }
  .ghost { height: 30px; padding: 0 10px; border-radius: 8px; border: 1px solid var(--bg-4); background: transparent; color: var(--text-2); font-size: 12px; }
  .content { flex: 1; min-width: 0; background: var(--bg-2); display: grid; place-items: center; }
  .members { width: 240px; background: var(--bg-1); }
  .empty { color: var(--text-3); }
  .error { color: #f2616b; }
</style>
