<script lang="ts">
  // Save as wheel (#158): keep a quick wheel's options as a saved wheel, Server (anyone in it) or Just me, under a
  // name. Shown in the wheel popover and from a quick spin's line. In a DM or group there is no server: Just me.
  import Segmented from './Segmented.svelte'
  import { errorText, wheelCmd } from '../lib/tauri'
  import { upsertWheel } from '../lib/wheels.svelte'
  import { WHEEL_LIMITS } from '../lib/wheel'
  import type { Wheel } from '../lib/protocol/Wheel'

  let { serverId, options, onSaved, onCancel }: {
    serverId: string | null
    options: string[]
    onSaved: (w: Wheel) => void
    onCancel: () => void
  } = $props()

  const SCOPES: { value: 'server' | 'mine'; label: string }[] = [{ value: 'server', label: 'Server' }, { value: 'mine', label: 'Just me' }]
  let scope = $state<'server' | 'mine'>('server')
  let name = $state('')
  let error = $state('')
  let saving = $state(false)
  const focusOnShow = (el: HTMLElement) => el.focus()

  async function save(e: SubmitEvent) {
    e.preventDefault()
    if (!name.trim() || saving) return
    saving = true
    error = ''
    try {
      const w = await wheelCmd.create(serverId && scope === 'server' ? serverId : null, name.trim(), options)
      upsertWheel(w)
      onSaved(w)
    } catch (err) {
      error = errorText(err)
    } finally {
      saving = false
    }
  }
</script>

<form class="save" onsubmit={save} aria-label="Save as wheel">
  {#if serverId}
    <Segmented label="Who sees it" options={SCOPES} value={scope}
      onchange={(v) => (scope = v)} />
  {/if}
  <input class="input input-sm" bind:value={name} maxlength={WHEEL_LIMITS.nameChars} placeholder="Name the wheel" aria-label="Wheel name"
    {@attach focusOnShow} onkeydown={(e) => { if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); onCancel() } }} />
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  <div class="btns">
    <button type="button" class="btn btn-secondary" onclick={onCancel}>Cancel</button>
    <button type="submit" class="btn btn-primary" disabled={!name.trim() || saving}>Save</button>
  </div>
</form>

<style>
  .save { display: flex; flex-direction: column; gap: 8px; }
  .error { margin: 0; color: var(--danger-text); font-size: 12px; }
  .btns { display: flex; justify-content: flex-end; gap: 6px; }
</style>
