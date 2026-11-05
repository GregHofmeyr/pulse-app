<script lang="ts">
  // The crop dialog for a group icon (#56), open while a picked picture waits (lib/groupIcon.svelte.ts). The same
  // square crop as an avatar.
  import CropDialog from './CropDialog.svelte'
  import { applyGroupIcon, cancelGroupIcon, groupIcon } from '../lib/groupIcon.svelte'
</script>

{#if groupIcon.pending}
  {#key groupIcon.pending}
    <CropDialog file={groupIcon.pending.file} kind="avatar" onApply={(c) => void applyGroupIcon(c)} onCancel={cancelGroupIcon} />
  {/key}
{/if}
