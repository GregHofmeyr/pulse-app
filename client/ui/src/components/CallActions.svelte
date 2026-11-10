<script lang="ts">
  // The call's action row (#221, like Discord's activity row): Go Live and Soundboard, in the voice panel at the bottom
  // of the sidebar (VoiceStatus) and in a DM or group call's band. The call view's own controls keep their buttons too.
  // The soundboard's dialogs live outside its popover (the popover's click-outside would close it under them), so
  // opening one closes the popover; leaving the call closes both.
  import Icon from './Icon.svelte'
  import SoundboardPopover from './SoundboardPopover.svelte'
  import SoundDialog from './SoundDialog.svelte'
  import SoundTidy from './SoundTidy.svelte'
  import type { Sound } from '../lib/protocol/Sound'
  import { voice } from '../lib/voice.svelte'
  import { pickSource } from '../lib/stream/golive.svelte'
  import { isWindows } from '../lib/stream/platform'
  import { stream, streamApi } from '../lib/stream/stream.svelte'

  let { channelId }: { channelId: string } = $props()

  const inHere = $derived(voice.channelId === channelId)
  const connected = $derived(inHere && voice.connection === 'connected')
  const live = $derived(stream.live?.channelId === channelId)
  // Go Live's reason to be off, if any (aria-disabled, not disabled: a disabled button swallows the hover that shows
  // the tip). A DM or group call streams like a server voice channel (#51).
  const off = $derived(
    !isWindows() ? "Streaming from Linux isn't supported yet"
    : !connected ? 'Wait until the call has connected'
    : null,
  )

  let sbOpen = $state(false)
  let sbBtn = $state<HTMLButtonElement>()
  let sbDialog = $state<{ kind: 'add'; serverId: string } | { kind: 'edit'; sound: Sound } | { kind: 'tidy'; serverId: string } | null>(null)
  const openDialog = (d: NonNullable<typeof sbDialog>) => {
    sbOpen = false
    sbDialog = d
  }
  $effect(() => {
    if (!inHere) {
      sbOpen = false
      sbDialog = null
    }
  })
</script>

<div class="call-actions" role="group" aria-label="Call actions">
  <!-- icon-only (owner, 2026-11-06): the name is the button's label and its tooltip -->
  {#if live}
    <button type="button" class="btn btn-primary" aria-label="Stop streaming" data-tip="Stop streaming" onclick={() => streamApi.stop()}><Icon name="screen" size={16} /></button>
  {:else}
    <button type="button" class="btn btn-quiet" aria-label="Go Live" aria-disabled={off ? 'true' : undefined} data-tip={off ?? 'Go Live'} onclick={() => { if (!off) void pickSource(channelId) }}><Icon name="screen" size={16} /></button>
  {/if}
  <button bind:this={sbBtn} type="button" class="btn btn-quiet" class:open={sbOpen} aria-label="Soundboard" data-tip="Soundboard" aria-haspopup="dialog" aria-expanded={sbOpen} data-menu-toggle onclick={() => (sbOpen = !sbOpen)}><Icon name="soundboard" size={16} /></button>
</div>

{#if sbOpen && sbBtn}
  <SoundboardPopover
    anchor={sbBtn}
    onClose={() => (sbOpen = false)}
    onAdd={(serverId) => openDialog({ kind: 'add', serverId })}
    onEdit={(sound) => openDialog({ kind: 'edit', sound })}
    onTidy={(serverId) => openDialog({ kind: 'tidy', serverId })}
  />
{/if}

{#if sbDialog?.kind === 'tidy'}
  <SoundTidy serverId={sbDialog.serverId} onClose={() => (sbDialog = null)} />
{:else if sbDialog}
  <SoundDialog mode={sbDialog} onClose={() => (sbDialog = null)} />
{/if}

<style>
  .call-actions { display: flex; gap: 8px; }
  /* two equal halves, icons only: a wide target that never wraps */
  .call-actions .btn { flex: 1 1 0; min-width: 0; padding: 0 8px; }
  .call-actions .btn.open { background: var(--bg-4); color: var(--text); }
</style>
