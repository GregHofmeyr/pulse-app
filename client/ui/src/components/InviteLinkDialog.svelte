<script lang="ts">
  // An invite opened from a `pulse://invite/<code>` link (#476: the invite page's "Open in Pulse"): its card, with its
  // own Join (or Open, once you're in). Nothing happens until you press it.
  import { modal } from '../lib/actions'
  import ServerInviteCard from './ServerInviteCard.svelte'

  let { code, onClose }: { code: string; onClose: () => void } = $props()
</script>

<div class="overlay modal-overlay fade-in">
  <div class="backdrop modal-backdrop" role="presentation" data-backdrop onclick={onClose}></div>
  <div class="modal pop-in" role="dialog" aria-modal="true" aria-label="Server invite" tabindex="-1" use:modal={onClose}>
    <h1 class="modal-title">You've been invited</h1>
    <ServerInviteCard {code} />
    <div class="actions">
      <button type="button" class="btn btn-lg btn-secondary" onclick={onClose}>Close</button>
    </div>
  </div>
</div>

<style>
  .overlay { position: fixed; inset: 0; z-index: 30; }
  .modal { width: min(420px, 100%); }
  .actions { display: flex; justify-content: flex-end; }
</style>
