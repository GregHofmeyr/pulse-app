<script lang="ts">
  // A message's view-once files (#311). For their sender: the files show as usual (MessageFiles), and this adds a
  // line saying who has opened them. For anyone else: no preview, just a card ("View once: photo", Open); opening
  // shows them in a dialog that says plainly it isn't screenshot-proof, and once it's closed they're gone for you: the
  // server signs no other link to them for you, on any device. Opened, the card says so.
  import type { Attachment } from '../lib/protocol/Attachment'
  import type { Message } from '../lib/protocol/Message'
  import type { AttachmentUrl } from '../lib/protocol/AttachmentUrl'
  import { loadViewOnce, openViewOnce, viewOnce } from '../lib/viewOnce.svelte'
  import { app } from '../lib/store.svelte'
  import { displayName } from '../lib/selectors'
  import { errorText } from '../lib/tauri'
  import { modal } from '../lib/actions'
  import Icon from './Icon.svelte'

  let { message, files, mine, onError }: { message: Message; files: Attachment[]; mine: boolean; onError?: (text: string) => void } = $props()

  $effect(() => loadViewOnce(message.id))
  const st = $derived(viewOnce.states[message.id])
  const serverId = $derived(app.state.channels[message.channel_id]?.server_id ?? null)
  const viewers = $derived((st?.viewers ?? []).map((v) => displayName(app.state, serverId, v.user_id)))
  const what = $derived.by(() => {
    if (files.length > 1) return `${files.length} files`
    const k = files[0]?.kind
    return k === 'image' ? 'photo' : k === 'video' ? 'video' : k === 'audio' ? 'sound' : 'file'
  })

  let open = $state<{ file: Attachment; url: string }[] | null>(null)
  let busy = $state(false)
  async function view() {
    if (busy) return
    busy = true
    try {
      const urls: AttachmentUrl[] = await openViewOnce(message.id)
      open = files.flatMap((f) => {
        const url = urls.find((u) => u.id === f.id)?.url
        return url ? [{ file: f, url }] : []
      })
    } catch (e) {
      onError?.(errorText(e))
    } finally {
      busy = false
    }
  }
</script>

{#if mine}
  <p class="once-line" data-view-once-status><Icon name="eyeOff" size={12} />View once · {viewers.length ? `Opened by ${viewers.join(', ')}` : 'Not opened yet'}</p>
{:else if st?.opened && !open}
  <div class="once-card opened" data-view-once><Icon name="eyeOff" size={16} /><span>View once {what} · Opened</span></div>
{:else if !open}
  <button type="button" class="once-card" data-view-once disabled={busy || !st} onclick={() => void view()}>
    <Icon name="eye" size={16} /><span>View once {what}</span><span class="go">Open</span>
  </button>
{/if}

{#if open}
  <div class="overlay modal-overlay fade-in">
    <div class="backdrop modal-backdrop" role="presentation" data-backdrop onclick={() => (open = null)}></div>
    <div class="modal pop-in" role="dialog" aria-modal="true" aria-label="View once" tabindex="-1" use:modal={() => (open = null)}>
      <h2 class="modal-title">View once</h2>
      <p class="warn">Closing this hides it for good. It isn't screenshot-proof: anyone can still take a screenshot.</p>
      <div class="media">
        {#each open as o (o.file.id)}
          {#if o.file.kind === 'image'}
            <img src={o.url} alt={o.file.name} draggable="false" oncontextmenu={(e) => e.preventDefault()} />
          {:else if o.file.kind === 'video'}
            <!-- svelte-ignore a11y_media_has_caption -->
            <video src={o.url} controls autoplay controlslist="nodownload" disablepictureinpicture oncontextmenu={(e) => e.preventDefault()}></video>
          {:else if o.file.kind === 'audio'}
            <audio src={o.url} controls autoplay controlslist="nodownload"></audio>
          {:else}
            <p class="file"><Icon name="file" size={14} /> {o.file.name}</p>
          {/if}
        {:else}
          <p class="file">It's no longer available.</p>
        {/each}
      </div>
      <div class="actions"><button type="button" class="btn btn-primary" onclick={() => (open = null)}>Close</button></div>
    </div>
  </div>
{/if}

<style>
  .once-line { margin: 4px 0 0; display: flex; align-items: center; gap: 6px; color: var(--text-3); font-size: 12px; }
  .once-card { margin-top: 6px; max-width: 432px; box-sizing: border-box; padding: 10px 12px; display: flex; align-items: center; gap: 8px;
    border-radius: var(--radius-xs); border: 1px solid var(--line-soft); background: var(--bg-2); color: var(--text); font: inherit; font-size: 13px; text-align: left;
    transition: background-color var(--dur-fast) var(--ease); }
  button.once-card:hover:not(:disabled) { background: var(--bg-3); }
  .once-card .go { margin-left: auto; color: var(--accent-text); font-weight: 600; }
  .once-card.opened { color: var(--text-3); }
  .overlay { position: fixed; inset: 0; z-index: 40; padding: 32px 16px; }
  .modal { width: min(720px, calc(100vw - 32px)); max-height: calc(100vh - 64px); gap: 10px; }
  .warn { margin: 0; color: var(--text-2); font-size: 13px; }
  .media { min-height: 0; overflow: auto; display: flex; flex-direction: column; gap: 8px; align-items: center; }
  .media img, .media video { max-width: 100%; max-height: 60vh; border-radius: var(--radius-xs); user-select: none; }
  .media audio { width: 100%; }
  .file { margin: 0; color: var(--text-2); font-size: 13px; }
  .actions { display: flex; justify-content: flex-end; }
</style>
