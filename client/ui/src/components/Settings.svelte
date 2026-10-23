<script lang="ts">
  import { onMount } from 'svelte'
  import Icon from './Icon.svelte'
  import VoiceSettings from './VoiceSettings.svelte'
  import KeybindSettings from './KeybindSettings.svelte'
  import ChatSettings from './ChatSettings.svelte'

  type Section = 'voice' | 'keybinds' | 'chat'
  let { onClose, section = 'voice' }: { onClose: () => void; section?: Section } = $props()

  let current = $state<Section>('voice')
  onMount(() => (current = section))
  const sections: { key: Section; label: string }[] = [
    { key: 'voice', label: 'Voice & Audio' },
    { key: 'keybinds', label: 'Keybinds' },
    { key: 'chat', label: 'Chat' },
  ]
</script>

<div class="overlay">
<div class="backdrop" role="presentation" onclick={onClose}></div>
<div class="modal" role="dialog" aria-label="Settings">
  <nav aria-label="Settings sections">
    <h1>Settings</h1>
    {#each sections as s}
      <button class:on={current === s.key} aria-current={current === s.key ? 'page' : undefined}
        onclick={() => (current = s.key)}>{s.label}</button>
    {/each}
  </nav>
  <div class="content">
    <button class="close" aria-label="Close settings" onclick={onClose}><Icon name="x" size={16} /></button>
    {#if current === 'voice'}<VoiceSettings />{:else if current === 'keybinds'}<KeybindSettings />{:else}<ChatSettings />{/if}
  </div>
</div>
</div>

<style>
  /* flex-centred (not transform): WebKitGTK blurs text on sub-pixel transforms */
  .overlay { position: fixed; inset: 0; z-index: 20; display: grid; place-items: center; padding: 32px 16px; }
  .backdrop { position: absolute; inset: 0; background: rgba(10, 11, 13, .6); }
  .modal { position: relative; width: min(940px, calc(100vw - 32px)); height: min(720px, calc(100vh - 64px)); display: flex; background: var(--bg-1); border-radius: 16px; border: 1px solid var(--bg-3); overflow: hidden; }
  nav { width: 200px; flex-shrink: 0; padding: 24px 12px; background: var(--bg-0); display: flex; flex-direction: column; gap: 2px; }
  h1 { margin: 0 10px 12px; font-size: 12px; font-weight: 700; letter-spacing: .06em; text-transform: uppercase; color: var(--text-3); }
  nav button { text-align: left; padding: 8px 10px; border: 0; border-radius: 8px; background: transparent; color: var(--text-2); font-weight: 600; }
  nav button:hover { background: var(--bg-2); color: var(--text); }
  nav button.on { background: var(--bg-3); color: var(--text); }
  .content { position: relative; flex: 1; min-width: 0; overflow-y: auto; padding: 28px 32px; }
  .close { position: absolute; top: 20px; right: 20px; width: 36px; height: 36px; border: 1px solid var(--bg-4); border-radius: 50%; background: transparent; color: var(--text-2); display: grid; place-items: center; z-index: 1; }
  @media (max-width: 640px) {
    .modal { flex-direction: column; }
    nav { width: auto; flex-direction: row; padding: 12px; }
    h1 { display: none; }
  }
</style>
