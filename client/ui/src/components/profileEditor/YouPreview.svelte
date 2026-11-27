<script lang="ts">
  // You, as the popovers' live preview shows you (#727): on your card, in a chat line and in the channel list. It reads
  // the editor's draft, so every choice shows at once. Names are drawn still here (nothing animates in a preview).
  import Avatar from '../Avatar.svelte'
  import Name from '../Name.svelte'
  import ProfileCard from '../ProfileCard.svelte'
  import { previewCard, previewUser } from '../../lib/profileDraft'
  import type { ProfileEditorState } from '../../lib/profileEditor.svelte'
  import { app } from '../../lib/store.svelte'

  let { state: st }: { state: ProfileEditorState } = $props()
  const me = $derived(app.state.me)
  const pu = $derived(me ? previewUser(me, st.draft) : null)
  const pc = $derived(previewCard(st.draft))
</script>

{#if me && pu}
  <div class="you">
    <ProfileCard user={pu} card={pc} badges={st.profile.badges} showcase={st.shown} stats={st.profile.stats} serverId={null} preview />
    <div class="line" role="group" aria-label="In chat">
      <Avatar id={me.id} name={pu.display_name} user={pu} size={32} />
      <div class="said">
        <Name userId={me.id} serverId={null} user={pu} text={pu.display_name} interactive={false} />
        <p>Looking good!</p>
      </div>
    </div>
    <div class="row" role="group" aria-label="In the channel list">
      <Avatar id={me.id} name={pu.display_name} user={pu} size={24} />
      <span class="who"><Name userId={me.id} serverId={null} user={pu} text={pu.display_name} interactive={false} surface="side" /></span>
    </div>
  </div>
{/if}

<style>
  .you { display: flex; flex-direction: column; align-items: center; gap: 12px; min-width: 0; max-width: 100%; }
  .line { align-self: stretch; display: flex; gap: 10px; align-items: flex-start; min-width: 0; padding: 10px; border-radius: var(--radius-lg); background: var(--bg-2); }
  .said { min-width: 0; display: flex; flex-direction: column; font-weight: 600; overflow: hidden; }
  .said p { margin: 2px 0 0; font-weight: 400; color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .row { align-self: stretch; display: flex; align-items: center; gap: 8px; min-width: 0; padding: 6px 10px; border-radius: var(--radius-md); background: var(--bg-3); }
  .who { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 600; }
</style>
