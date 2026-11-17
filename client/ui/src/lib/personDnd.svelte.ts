// Per-person Do Not Disturb (#310), the client side: each change asks the server, which answers with your whole list
// (and tells your other devices with PersonDndChanged); the answer replaces ours.
import type { PersonDndRequest } from './protocol/PersonDndRequest'
import { app } from './store.svelte'
import { api } from './tauri'

/** Turn it on for `userId` (or change it), or off with null. */
export async function setPersonDnd(userId: string, request: PersonDndRequest | null): Promise<void> {
  const list = await api.setPersonDnd(userId, request)
  app.state = { ...app.state, personDnd: list }
}
