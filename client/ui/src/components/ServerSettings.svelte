<script lang="ts">
  import Icon from './Icon.svelte'
  import { modal } from '../lib/actions'
  import EmojiSettings from './EmojiSettings.svelte'
  import SoundboardSettings from './SoundboardSettings.svelte'
  import WheelSettings from './WheelSettings.svelte'
  import StorageSettings from './StorageSettings.svelte'
  import SpeakerLinkSettings from './SpeakerLinkSettings.svelte'
  import RecentlyDeleted from './RecentlyDeleted.svelte'
  import NotifyLevelSettings from './NotifyLevelSettings.svelte'
  import DeleteServer from './DeleteServer.svelte'
  import LeaveServer from './LeaveServer.svelte'
  import TransferOwnership from './TransferOwnership.svelte'
  import ServerMembers from './ServerMembers.svelte'
  import ServerInvites from './ServerInvites.svelte'
  import ServerBans from './ServerBans.svelte'
  import { isMember } from '../lib/selectors'
  import AchievementsChannelSettings from './AchievementsChannelSettings.svelte'
  import F1ChannelSettings from './F1ChannelSettings.svelte'
  import SportsChannelSettings from './SportsChannelSettings.svelte'
  import DeadlockChannelSettings from './DeadlockChannelSettings.svelte'
  import { app } from '../lib/store.svelte'
  import type { Server } from '../lib/protocol/Server'

  let { server, onClose }: { server: Server; onClose: () => void } = $props()
  // Notifications (#203): your own level for the server, as its tab's right-click menu sets it. Members (#418): everyone
  // in it, each with their person menu. Invites (#80): its members' (the owner sees all). Bans (#80): the owner's. Leave server (#371): for anyone in it (its owner is told to hand it over
  // first). Transfer ownership (#418) and Delete server (#184): its owner's only, last.
  type Section = 'Emoji' | 'Soundboard' | 'Wheels' | 'Quote speakers' | 'Achievements' | 'Deadlock' | 'F1' | 'Sports' | 'Notifications' | 'Storage' | 'Recently deleted' | 'Members' | 'Invites' | 'Bans' | 'Leave server' | 'Transfer ownership' | 'Delete server'
  const owner = $derived(!!server.owner_id && server.owner_id === app.state.me?.id)
  const sections = $derived<Section[]>([
    'Emoji', 'Soundboard', 'Wheels', 'Quote speakers', 'Achievements', 'Deadlock', 'F1', 'Sports', 'Notifications', 'Storage', 'Recently deleted', 'Members',
    ...(isMember(app.state, server.id) ? (['Invites'] as const) : []),
    ...(owner ? (['Bans'] as const) : []),
    ...(isMember(app.state, server.id) ? (['Leave server'] as const) : []),
    ...(owner ? (['Transfer ownership', 'Delete server'] as const) : []),
  ])
  let section = $state<Section>('Emoji')
  // Handed over while on an owner's section: Leave server is what's left to do there
  $effect(() => {
    if (!sections.includes(section)) section = sections.includes('Leave server') ? 'Leave server' : 'Emoji'
  })
</script>

<div class="overlay modal-overlay fade-in">
<div class="backdrop modal-backdrop" role="presentation" data-backdrop onclick={onClose}></div>
<div class="modal pop-in" role="dialog" aria-modal="true" aria-label="Server settings" tabindex="-1" use:modal={onClose}>
  <nav aria-label="Server settings sections">
    <h1>{server.name}</h1>
    {#each sections as s (s)}
      <button class:on={section === s} class:danger={s === 'Delete server' || s === 'Leave server' || s === 'Transfer ownership'} aria-current={section === s ? 'page' : undefined} onclick={() => (section = s)}>{s}</button>
    {/each}
  </nav>
  <div class="content">
    <button class="close icon-btn round modal-close" aria-label="Close server settings" onclick={onClose}><Icon name="x" size={16} /></button>
    {#if section === 'Members'}<ServerMembers {server} />{:else if section === 'Invites'}<ServerInvites {server} />{:else if section === 'Bans' && owner}<ServerBans {server} />{:else if section === 'Emoji'}<EmojiSettings {server} />{:else if section === 'Soundboard'}<SoundboardSettings {server} />{:else if section === 'Wheels'}<WheelSettings {server} />{:else if section === 'Achievements'}<AchievementsChannelSettings {server} />{:else if section === 'Deadlock'}<DeadlockChannelSettings {server} />{:else if section === 'F1'}<F1ChannelSettings {server} />{:else if section === 'Sports'}<SportsChannelSettings {server} />{:else if section === 'Notifications'}<NotifyLevelSettings {server} />{:else if section === 'Storage'}<StorageSettings {server} />{:else if section === 'Recently deleted'}<RecentlyDeleted {server} />{:else if section === 'Leave server'}<LeaveServer {server} {owner} onLeft={onClose} />{:else if section === 'Transfer ownership' && owner}<TransferOwnership {server} />{:else if section === 'Delete server' && owner}<DeleteServer {server} onDeleted={onClose} />{:else}<SpeakerLinkSettings {server} />{/if}
  </div>
</div>
</div>

<style>
  .overlay { position: fixed; inset: 0; z-index: 20; padding: 32px 16px; }
  /* a window like Settings: bg-1, nav and content set their own padding */
  .modal { width: min(940px, calc(100vw - 32px)); height: min(720px, calc(100vh - 64px)); flex-direction: row; gap: 0; padding: 0; background: var(--bg-1); overflow: hidden; }
  nav { width: 200px; flex-shrink: 0; padding: 24px 12px; background: var(--bg-0); display: flex; flex-direction: column; gap: 2px; }
  h1 { margin: 0 10px 12px; font-size: 12px; font-weight: 700; letter-spacing: .06em; text-transform: uppercase; color: var(--text-3); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  nav button { text-align: left; padding: 8px 10px; border: 0; border-radius: var(--radius-sm); background: transparent; color: var(--text-2); font-weight: 600; }
  nav button:hover { background: var(--bg-2); color: var(--text); }
  nav button.on { background: var(--bg-3); color: var(--text); }
  nav button.danger { color: var(--danger-text); }
  .content { position: relative; flex: 1; min-width: 0; overflow-y: auto; padding: 28px 32px; }
  .close { position: absolute; top: 20px; right: 20px; z-index: 1; }
  @media (max-width: 640px) {
    .modal { flex-direction: column; }
    nav { width: auto; flex-direction: row; padding: 12px; }
    h1 { display: none; }
  }
</style>
