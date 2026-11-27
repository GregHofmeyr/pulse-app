<script lang="ts" module>
  export type Section = 'voice' | 'keybinds' | 'chat' | 'wheels' | 'notifications' | 'privacy' | 'sports' | 'deleted' | 'advanced' | 'about'
</script>

<script lang="ts">
  import { untrack } from 'svelte'
  import Icon from './Icon.svelte'
  import { modal } from '../lib/actions'
  import VoiceSettings from './VoiceSettings.svelte'
  import KeybindSettings from './KeybindSettings.svelte'
  import ChatSettings from './ChatSettings.svelte'
  import NotificationSettings from './NotificationSettings.svelte'
  import AdvancedSettings from './AdvancedSettings.svelte'
  import { openProfileEditor } from '../lib/profileEditor.svelte'
  import PrivacySettings from './PrivacySettings.svelte'
  import WheelSettings from './WheelSettings.svelte'
  import DeletedServers from './DeletedServers.svelte'
  import SportsSettings from './SportsSettings.svelte'
  import AboutSettings from './AboutSettings.svelte'
  let { onClose, onLogout, section = 'voice' }: {
    onClose: () => void
    /** Log out (#168): the last item of the navigation, under the sections, as on Discord. */
    onLogout?: () => void
    section?: Section
  } = $props()

  // the section asked for from the start: not Voice first, mounted and thrown away a moment later (#238)
  let current = $state<Section>(untrack(() => section))
  const sections: { key: Section; label: string }[] = [
    { key: 'voice', label: 'Voice & Audio' },
    { key: 'keybinds', label: 'Keybinds' },
    { key: 'chat', label: 'Chat' },
    // your Just me picker wheels (#158); a server's own are in its Server settings
    { key: 'wheels', label: 'Wheels' },
    { key: 'notifications', label: 'Notifications' },
    { key: 'privacy', label: 'Privacy' },
    // sports score pings (#385): your teams and the switch
    { key: 'sports', label: 'Sports' },
    // the servers you deleted (#184): a deleted server has no Server settings to hold its Recently deleted
    { key: 'deleted', label: 'Recently deleted' },
    { key: 'advanced', label: 'Advanced' },
    // which Pulse this is, where it's connected, updates and this version's notes (#506)
    { key: 'about', label: 'About' },
  ]

  // Profile is the editor's now (#727): Settings closes, then the editor opens on Looks
  function openProfile() {
    onClose()
    openProfileEditor()
  }
</script>

<div class="overlay modal-overlay fade-in">
<div class="backdrop modal-backdrop" role="presentation" data-backdrop onclick={onClose}></div>
<div class="modal pop-in" role="dialog" aria-modal="true" aria-label="Settings" tabindex="-1" use:modal={onClose}>
  <nav aria-label="Settings sections">
    <h1>Settings</h1>
    <button onclick={openProfile}>Profile</button>
    {#each sections as s}
      <button class:on={current === s.key} aria-current={current === s.key ? 'page' : undefined} onclick={() => (current = s.key)}>{s.label}</button>
    {/each}
    {#if onLogout}
      <span class="nav-sep" role="separator"></span>
      <button type="button" class="danger" onclick={onLogout}>Log out</button>
    {/if}
  </nav>
  <div class="content">
    <button class="close icon-btn round modal-close" aria-label="Close settings" onclick={onClose}><Icon name="x" size={16} /></button>
    {#if current === 'voice'}<VoiceSettings />{:else if current === 'keybinds'}<KeybindSettings />{:else if current === 'chat'}<ChatSettings />{:else if current === 'wheels'}<WheelSettings />{:else if current === 'notifications'}<NotificationSettings />{:else if current === 'privacy'}<PrivacySettings />{:else if current === 'sports'}<SportsSettings />{:else if current === 'deleted'}<DeletedServers />{:else if current === 'about'}<AboutSettings />{:else}<AdvancedSettings />{/if}
  </div>
</div>
</div>

<style>
  .overlay { position: fixed; inset: 0; z-index: 20; padding: 32px 16px; }
  /* a window, not a dialog: bg-1 (the panes are drawn for it), nav and content set their own padding */
  .modal { width: min(940px, calc(100vw - 32px)); height: min(720px, calc(100vh - 64px)); flex-direction: row; gap: 0; padding: 0; background: var(--bg-1); overflow: hidden; }
  nav { width: 200px; flex-shrink: 0; padding: 24px 12px; background: var(--bg-0); display: flex; flex-direction: column; gap: 2px; }
  h1 { margin: 0 10px 12px; font-size: 12px; font-weight: 700; letter-spacing: .06em; text-transform: uppercase; color: var(--text-3); }
  nav button { text-align: left; padding: 8px 10px; border: 0; border-radius: var(--radius-sm); background: transparent; color: var(--text-2); font-weight: 600; }
  nav button:hover { background: var(--bg-2); color: var(--text); }
  nav button.on { background: var(--bg-3); color: var(--text); }
  /* Log out (#168): set apart under the last section, in the danger colour */
  .nav-sep { height: 1px; margin: 8px 10px; background: var(--line-soft); }
  nav button.danger { color: var(--danger-text); }
  nav button.danger:hover { color: var(--danger-text); }
  .content { position: relative; flex: 1; min-width: 0; overflow-y: auto; padding: 28px 32px; }
  .close { position: absolute; top: 20px; right: 20px; z-index: 1; }
  @media (max-width: 640px) {
    .modal { flex-direction: column; }
    nav { width: auto; flex-direction: row; padding: 12px; }
    h1 { display: none; }
    .nav-sep { width: 1px; height: auto; margin: 4px; }
  }
</style>
