<script lang="ts">
  // A file in a message that isn't shown inline (#57): icon, name, size and Download, on the link preview's card
  // (LinkEmbed). Files that can run programs say so beside Download. Pulse saves files; it never opens them. A temporary
  // send says when it expires, and once it has, Expired (its bytes are gone).
  import Icon from './Icon.svelte'
  import type { Attachment } from '../lib/protocol/Attachment'
  import { expiresLabel, formatSize, isExecutable, isExpired, RUNS_PROGRAMS } from '../lib/attachments'
  import { api, errorText } from '../lib/tauri'
  import { clock } from '../lib/store.svelte'

  let { file, messageId, onError }: {
    file: Attachment
    messageId: string
    onError?: (text: string) => void
  } = $props()

  const runs = $derived(isExecutable(file.name))
  const now = $derived(Date.parse(clock.nowIso))
  const expired = $derived(isExpired(file, now))
  const icon = $derived(file.kind === 'video' ? 'film' : file.kind === 'audio' ? 'music' : file.kind === 'image' ? 'image' : 'file')
  let saving = $state(false)

  async function download() {
    if (saving) return
    saving = true
    try {
      await api.saveAttachment(messageId, file.id, file.name)
    } catch (e) {
      onError?.(errorText(e))
    } finally {
      saving = false
    }
  }
</script>

<div class="card">
  <span class="glyph"><Icon name={icon} size={18} /></span>
  <span class="text">
    <span class="name">{file.name}</span>
    <span class="size">{formatSize(file.size)}{#if file.expires_at && !expired}{` · ${expiresLabel(file.expires_at, now)}`}{/if}</span>
  </span>
  {#if expired}
    <span class="expired">Expired</span>
  {:else}
    <button class="btn btn-quiet download" type="button" aria-label={`Download ${file.name}`} disabled={saving} onclick={download}>
      <Icon name="download" size={14} /> Download
    </button>
  {/if}
  {#if runs && !expired}
    <p class="warn"><Icon name="alert" size={14} /> {RUNS_PROGRAMS}</p>
  {/if}
</div>

<style>
  /* the link preview's card (LinkEmbed .embed) */
  .card {
    display: grid; grid-template-columns: auto minmax(0, 1fr) auto; align-items: center; gap: 4px 12px;
    max-width: 432px; box-sizing: border-box; margin-top: 6px; padding: 10px 12px;
    border-radius: var(--radius-xs); background: var(--bg-2); border: 1px solid var(--line-soft);
  }
  .glyph { width: 36px; height: 36px; border-radius: var(--radius-xs); display: grid; place-items: center; color: var(--text-3); background: var(--bg-3); }
  .text { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .name { font-size: 14px; font-weight: 600; color: var(--text); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .size { font-size: 12px; color: var(--text-3); }
  .download { height: 28px; padding: 0 10px; font-size: 12px; display: inline-flex; align-items: center; gap: 6px; }
  .expired { font-size: 12px; font-weight: 600; color: var(--text-3); }
  .warn { grid-column: 1 / -1; margin: 4px 0 0; display: flex; align-items: center; gap: 6px; font-size: 12px; color: var(--warn-text); }
</style>
