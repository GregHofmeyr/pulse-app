<script lang="ts">
  import { getCurrentWindow } from '@tauri-apps/api/window'
  import Icon from '../components/Icon.svelte'
  import ChannelList from '../components/ChannelList.svelte'
  import MemberList from '../components/MemberList.svelte'
  import VoicePanel from '../components/VoicePanel.svelte'
  import TextChannel from '../components/TextChannel.svelte'
  import Settings from '../components/Settings.svelte'
  import ConversationList from '../components/ConversationList.svelte'
  import PeopleBoard from '../components/PeopleBoard.svelte'
  import PeoplePicker from '../components/PeoplePicker.svelte'
  import { onMount } from 'svelte'
  import { voice, voiceApi } from '../lib/voice.svelte'
  import { api, errorText } from '../lib/tauri'
  import { app, openDm, ui } from '../lib/store.svelte'
  import { homeBadge, serverBadge } from '../lib/conversations'
  import { isMember } from '../lib/selectors'
  import { avatarColor, initial } from '../lib/avatar'
  import type { User } from '../lib/protocol/User'

  let { user, onLogout }: { user: User; onLogout: () => void } = $props()

  const win = getCurrentWindow()
  let activeServerId = $state<string | null>(null)
  let activeChannelId = $state<string | null>(null)
  let settingsOpen = $state(false)
  let menuOpen = $state(false)
  let creating = $state(false)
  let newName = $state('')
  let createError = $state('')
  /** People picker: a new DM/group from Home, or Add people in a conversation. */
  let picker = $state<null | { mode: 'new' } | { mode: 'add'; channelId: string }>(null)
  // Badges depend on mute expiry: re-evaluate every 30 s.
  let nowIso = $state(new Date().toISOString())
  onMount(() => {
    const t = setInterval(() => (nowIso = new Date().toISOString()), 30_000)
    const focus = () => (ui.focused = true)
    const blur = () => (ui.focused = false)
    window.addEventListener('focus', focus)
    window.addEventListener('blur', blur)
    ui.focused = document.hasFocus()
    return () => {
      clearInterval(t)
      window.removeEventListener('focus', focus)
      window.removeEventListener('blur', blur)
    }
  })
  $effect(() => {
    ui.openChannelId = activeChannelId
  })
  const home = $derived(homeBadge(app.state, nowIso))

  async function openConversation(id: string) {
    activeServerId = null
    activeChannelId = id
  }

  async function messageUser(userId: string) {
    try {
      await openConversation(await openDm(userId))
    } catch (e) {
      error = errorText(e)
    }
  }

  async function pickerDone(ids: string[]) {
    const p = picker
    picker = null
    if (!p) return
    try {
      if (p.mode === 'new') {
        await openConversation(ids.length === 1 ? await openDm(ids[0]) : (await api.createDm(ids)).id)
      }
    } catch (e) {
      error = errorText(e)
    }
  }

  async function createServer(e: SubmitEvent) {
    e.preventDefault()
    createError = ''
    try {
      const s = await api.createServer(newName.trim())
      creating = false
      newName = ''
      // the ServerCreated/ChannelCreated events may still be in flight: select once they land
      const pick = () => (app.state.servers.some((x) => x.id === s.id) ? selectServer(s.id) : setTimeout(pick, 50))
      pick()
    } catch (err) {
      createError = errorText(err)
    }
  }
  let error = $state('')
  const voiceChannel = $derived(voice.channelId ? app.state.channels[voice.channelId] : null)

  const servers = $derived(app.state.servers)
  const activeServer = $derived(servers.find((s) => s.id === activeServerId) ?? null)
  const member = $derived(activeServerId ? isMember(app.state, activeServerId) : false)
  const activeChannel = $derived(activeChannelId ? app.state.channels[activeChannelId] : null)

  // Launch lands on Home (a clean slate); only the person who creates a server is moved into it.

  function selectServer(id: string) {
    activeServerId = id
    const general = Object.values(app.state.channels).find((c) => c.server_id === id && c.kind === 'text')
    activeChannelId = general?.id ?? null
  }

  async function join() {
    if (!activeServerId) return
    try {
      await api.joinServer(activeServerId)
    } catch (e) {
      error = errorText(e)
    }
  }

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
      <button class="tab" class:active={activeServerId === null} onclick={() => { activeServerId = null; activeChannelId = null }}>
        <Icon name="home" size={16} /> Home{#if home > 0}<span class="pill">{home > 99 ? '99+' : home}</span>{/if}
      </button>
      <span class="sep"></span>
      {#each servers as s (s.id)}
        {@const badge = serverBadge(app.state, s.id, nowIso)}
        <button class="tab" class:active={activeServerId === s.id} onclick={() => selectServer(s.id)}>
          <span class="chip" style:background={avatarColor(s.id)}>{initial(s.name)}</span>{s.name}
          {#if badge.mentions > 0}<span class="pill">{badge.mentions}</span>{:else if badge.dot}<span class="dot" aria-label="new messages"></span>{/if}
        </button>
      {/each}
      <button class="add" aria-label="Create a server" title="Create a server" onclick={() => (creating = true)}><Icon name="plus" size={16} /></button>
    </nav>
    <div class="grow" data-tauri-drag-region></div>
    {#if app.state.conn !== 'connected'}
      <span class="conn" role="status">{app.state.conn === 'reconnecting' ? 'Reconnecting…' : 'Connecting…'}</span>
    {/if}
    <button class="win" aria-label="Minimise" onclick={() => win.minimize()}><Icon name="min" size={14} /></button>
    <button class="win" aria-label="Maximise" onclick={() => win.toggleMaximize()}><Icon name="square" size={13} /></button>
    <button class="win" aria-label="Close" onclick={() => win.close()}><Icon name="x" size={14} /></button>
  </header>

  <div class="body">
    <aside class="panel side">
      <div class="panel-head">{activeServer ? activeServer.name : 'Direct messages'}</div>
      {#if activeServerId}
        <ChannelList serverId={activeServerId} {activeChannelId} voiceChannelId={voice.channelId} speaking={voice.speaking}
          onSelect={(id) => (activeChannelId = id)}
          onJoinVoice={(id) => { activeChannelId = id; if (voice.channelId !== id && member) void voiceApi.join(id) }} />
      {:else}
        <ConversationList activeId={activeChannelId} onOpen={openConversation} onNew={() => (picker = { mode: 'new' })} />
      {/if}
      {#if voiceChannel}
        <div class="vc">
          <span class="vc-bars" class:warn={voice.connection !== 'connected'}><span></span><span></span><span></span></span>
          <span class="vc-text"><strong class:warn={voice.connection !== 'connected'}>{voice.connection === 'connected' ? 'Voice connected' : voice.connection === 'reconnecting' ? 'Reconnecting…' : 'Connecting…'}</strong><small>{voiceChannel.name}</small></span>
          <button class="vc-leave" aria-label="Leave voice" onclick={() => voiceApi.leave()}><Icon name="hangup" size={17} /></button>
        </div>
      {/if}
      <div class="me">
        <button class="who-btn" data-tip="Account" aria-haspopup="menu" aria-expanded={menuOpen} onclick={() => (menuOpen = !menuOpen)}>
          <span class="avatar" style:background={avatarColor(user.id)}>{initial(user.username)}</span>
          <span class="who">{user.username}<small>{app.state.conn === 'connected' ? 'Online' : 'Offline'}</small></span>
        </button>
        {#if voice.channelId}
          <button class="me-btn" class:on={voice.controls.muted} aria-pressed={voice.controls.muted} aria-label={voice.controls.muted ? "Unmute" : "Mute"} onclick={() => voiceApi.toggleMute()}>
            <Icon name={voice.controls.muted ? 'micOff' : 'mic'} />
          </button>
          <button class="me-btn" class:on={voice.controls.deafened} aria-pressed={voice.controls.deafened} aria-label={voice.controls.deafened ? "Undeafen" : "Deafen"} onclick={() => voiceApi.toggleDeafen()}>
            <Icon name={voice.controls.deafened ? 'headphonesOff' : 'headphones'} />
          </button>
        {/if}
        <button class="me-btn" aria-label="Voice & audio settings" onclick={() => (settingsOpen = true)}><Icon name="gear" /></button>
        {#if menuOpen}
          <div class="menu" role="menu">
            <button role="menuitem" onclick={() => { menuOpen = false; settingsOpen = true }}>Voice &amp; audio settings</button>
            <button role="menuitem" class="danger" onclick={logout}>Log out</button>
          </div>
        {/if}
      </div>
    </aside>

    <main class="panel content">
      {#if activeServerId && !member}
        <div class="join">
          <p>You're not in <strong>{activeServer?.name}</strong> yet.</p>
          <button class="primary" onclick={join}>Join server</button>
          {#if error}<p class="error">{error}</p>{/if}
        </div>
      {:else if activeChannel?.kind === 'voice'}
        <VoicePanel channelId={activeChannel.id} serverId={activeServerId} />
      {:else if activeChannel}
        {#key activeChannel.id}<TextChannel channelId={activeChannel.id} serverId={activeServerId} />{/key}
      {:else if activeServerId === null}
        <PeopleBoard onOpen={openConversation} onJoinVoice={(ch, srv) => { selectServer(srv); activeChannelId = ch; if (voice.channelId !== ch) void voiceApi.join(ch) }} />
      {:else}
        <p class="empty">Pick a channel.</p>
      {/if}
    </main>

    {#if activeServerId}<MemberList serverId={activeServerId} onMessage={messageUser} />{/if}
  </div>
</div>
{#if settingsOpen}<Settings onClose={() => (settingsOpen = false)} />{/if}
{#if creating}
  <div class="overlay">
  <div class="backdrop" role="presentation" onclick={() => (creating = false)}></div>
  <form class="dialog" onsubmit={createServer} aria-label="Create a server">
    <h2>Create a server</h2>
    <label>NAME <input bind:value={newName} maxlength="64" required placeholder="Game Night" /></label>
    {#if createError}<p class="error">{createError}</p>{/if}
    <div class="dialog-actions">
      <button type="button" class="ghost" onclick={() => (creating = false)}>Cancel</button>
      <button class="primary" disabled={!newName.trim()}>Create</button>
    </div>
  </form>
  </div>
{/if}

{#if picker?.mode === 'new'}
  <PeoplePicker title="New message" hint="Pick one person for a DM, or several for a group." action="Start" onDone={pickerDone} onClose={() => (picker = null)} />
{/if}

<style>
  .app { height: 100%; display: flex; flex-direction: column; background: var(--bg-0); }
  header { height: 52px; flex-shrink: 0; display: flex; align-items: center; gap: 6px; padding: 0 10px 0 12px; }
  .logo { width: 30px; height: 30px; border-radius: 9px; background: var(--accent); color: var(--on-accent); display: grid; place-items: center; margin-right: 8px; }
  nav { display: flex; align-items: center; gap: 6px; min-width: 0; overflow: hidden; }
  .tab { height: 36px; padding: 0 12px 0 10px; border-radius: 10px; border: 1px solid transparent; background: transparent; color: var(--text-2); display: flex; align-items: center; gap: 8px; font-size: 13px; font-weight: 500; white-space: nowrap; }
  .tab:hover { background: var(--bg-2); }
  .tab.active { background: #262830; border-color: var(--bg-4); color: #f6f7f9; font-weight: 600; }
  .chip { width: 22px; height: 22px; border-radius: 7px; color: #fff; font-size: 11px; font-weight: 700; display: grid; place-items: center; }
  .sep { width: 1px; height: 20px; background: var(--line); margin: 0 4px; }
  .grow { flex: 1; align-self: stretch; }
  .conn { font-size: 12px; font-weight: 600; color: #e8b04a; padding: 4px 10px; border-radius: 12px; background: rgba(232, 176, 74, .12); margin-right: 6px; }
  .win { width: 32px; height: 28px; border: 0; border-radius: 7px; background: transparent; color: var(--text-3); display: grid; place-items: center; }
  .win:hover { background: var(--bg-2); color: var(--text); }
  .body { flex: 1; min-height: 0; display: flex; gap: 8px; padding: 0 8px 8px; }
  .panel { border-radius: var(--radius); overflow: hidden; }
  .side { width: 264px; flex-shrink: 0; background: var(--bg-1); display: flex; flex-direction: column; }
  .panel-head { height: 52px; flex-shrink: 0; padding: 0 16px; display: flex; align-items: center; font-size: 15px; font-weight: 600; border-bottom: 1px solid #26282e; }
  .me { padding: 10px 10px 10px 12px; background: #17181c; display: flex; align-items: center; gap: 10px; }
  .avatar { width: 34px; height: 34px; border-radius: 50%; color: #fff; font-weight: 700; display: grid; place-items: center; }
  .who { flex: 1; display: flex; flex-direction: column; font-size: 13px; font-weight: 600; }
  .who small { font-size: 12px; font-weight: 400; color: var(--text-3); }
  .add { width: 36px; height: 36px; flex-shrink: 0; border: 1px dashed #3a3c44; border-radius: 10px; background: transparent; color: var(--text-3); display: grid; place-items: center; }
  .add:hover { color: var(--text); border-color: var(--text-3); }
  .me { position: relative; }
  .who-btn { flex: 1; min-width: 0; display: flex; align-items: center; gap: 10px; padding: 4px 6px 4px 0; border: 0; border-radius: 10px; background: transparent; text-align: left; }
  .who-btn:hover { background: var(--bg-2); }
  .who-btn .who { overflow: hidden; }
  .menu { position: absolute; left: 8px; bottom: calc(100% + 6px); z-index: 10; min-width: 200px; padding: 6px; background: #2e3037; border: 1px solid #3d4048; border-radius: 12px; box-shadow: 0 18px 48px rgba(8, 9, 12, .5); display: flex; flex-direction: column; }
  .menu button { height: 34px; padding: 0 10px; border: 0; border-radius: 8px; background: transparent; color: var(--text); text-align: left; font-size: 13px; }
  .menu button:hover { background: var(--bg-4); }
  .menu .danger { color: #f2616b; }
  .overlay { position: fixed; inset: 0; z-index: 20; display: grid; place-items: center; padding: 16px; }
  .backdrop { position: absolute; inset: 0; background: rgba(10, 11, 13, .6); }
  .dialog { position: relative; width: min(420px, calc(100vw - 32px)); padding: 24px; background: var(--bg-1); border: 1px solid var(--bg-3); border-radius: 16px; display: flex; flex-direction: column; gap: 14px; }
  .dialog h2 { margin: 0; font-size: 18px; }
  .dialog label { display: flex; flex-direction: column; gap: 6px; font-size: 12px; font-weight: 600; letter-spacing: .04em; color: var(--text-2); }
  .dialog input { height: 40px; padding: 0 12px; border-radius: 10px; border: 1px solid var(--bg-4); background: var(--bg-2); font-size: 14px; font-weight: 400; letter-spacing: 0; }
  .dialog-actions { display: flex; justify-content: flex-end; gap: 8px; }
  .me-btn { width: 32px; height: 32px; border: 0; border-radius: 8px; background: transparent; color: var(--text-2); display: grid; place-items: center; }
  .me-btn:hover { background: var(--bg-2); }
  .me-btn.on { color: #f2616b; }
  .vc { margin: 0 8px 8px; padding: 10px 10px 10px 14px; background: #232429; border-radius: 12px; display: flex; align-items: center; gap: 10px; }
  .vc-bars { display: flex; align-items: flex-end; gap: 2px; height: 14px; }
  .vc-bars span { width: 3px; border-radius: 1px; background: var(--ok); }
  .vc-bars span:nth-child(1) { height: 5px; } .vc-bars span:nth-child(2) { height: 9px; } .vc-bars span:nth-child(3) { height: 14px; }
  .vc-bars.warn span { background: #e8b04a; }
  .vc-text { flex: 1; display: flex; flex-direction: column; min-width: 0; }
  .vc-text strong { font-size: 13px; color: var(--ok); }
  .vc-text strong.warn { color: #e8b04a; }
  .vc-text small { font-size: 12px; color: var(--text-3); }
  .vc-leave { width: 34px; height: 34px; border: 0; border-radius: 9px; background: #2e3037; color: #f2616b; display: grid; place-items: center; }
  .ghost { height: 30px; padding: 0 10px; border-radius: 8px; border: 1px solid var(--bg-4); background: transparent; color: var(--text-2); font-size: 12px; }
  .content { flex: 1; min-width: 0; background: var(--bg-2); display: flex; flex-direction: column; }
  .empty { margin: auto; color: var(--text-3); }
  .join { margin: auto; display: flex; flex-direction: column; align-items: center; gap: 12px; }
  .primary { height: 40px; padding: 0 18px; border: 0; border-radius: 10px; background: var(--accent); color: var(--on-accent); font-weight: 600; }
  .error { color: #f2616b; }
  .tab .pill { margin-left: 4px; min-width: 16px; height: 16px; padding: 0 4px; border-radius: 8px; background: var(--danger); color: #fff; font-size: 10px; font-weight: 700; display: inline-grid; place-items: center; }
  .tab .dot { margin-left: 4px; width: 7px; height: 7px; border-radius: 50%; background: var(--text); display: inline-block; }
</style>
