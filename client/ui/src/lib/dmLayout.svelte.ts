// Your conversation list's pins and categories (#305), the client side: each change asks the server, which answers
// with your whole layout (and tells your other devices with DmLayoutChanged); the answer replaces ours. Folding a
// category shows at once and is put back if the server refuses.
import type { DmLayout } from './protocol/DmLayout'
import { placementOf } from './dmLayout'
import { app } from './store.svelte'
import { dmLayoutCmd } from './tauri'

const put = (layout: DmLayout): DmLayout => {
  app.state = { ...app.state, dmLayout: layout }
  return layout
}

const current = (channelId: string) => {
  const p = placementOf(app.state.dmLayout, channelId)
  return { pinned: p?.pinned ?? false, category_id: p?.category_id ?? null }
}

/** Pin a conversation to the top of your list, or unpin it (its category, if any, stays). */
export const pinConversation = async (channelId: string, pinned: boolean): Promise<DmLayout> =>
  put(await dmLayoutCmd.place(channelId, { ...current(channelId), pinned }))

/** Move a conversation into one of your categories, or out of any (`null`). Pinned stays as it was. */
export const moveConversation = async (channelId: string, categoryId: string | null): Promise<DmLayout> =>
  put(await dmLayoutCmd.place(channelId, { ...current(channelId), category_id: categoryId }))

/** A new category, last; resolves with its id. */
export async function createCategory(name: string): Promise<string> {
  const before = new Set(app.state.dmLayout.categories.map((c) => c.id))
  const layout = put(await dmLayoutCmd.create(name.trim()))
  const made = layout.categories.filter((c) => !before.has(c.id))
  return (made.at(-1) ?? layout.categories.at(-1))!.id
}

/** A new category with a conversation moved straight into it. */
export async function moveToNewCategory(channelId: string, name: string): Promise<void> {
  const id = await createCategory(name)
  await moveConversation(channelId, id)
}

export const renameCategory = async (id: string, name: string): Promise<DmLayout> => put(await dmLayoutCmd.update(id, { name: name.trim() }))

/** Delete a category: its conversations go back to the main list (pinned ones stay pinned). */
export const deleteCategory = async (id: string): Promise<DmLayout> => put(await dmLayoutCmd.remove(id))

/** Fold a category open or shut: at once, put back when refused. */
export async function setCategoryCollapsed(id: string, collapsed: boolean): Promise<void> {
  const was = app.state.dmLayout
  put({ ...was, categories: was.categories.map((c) => (c.id === id ? { ...c, collapsed } : c)) })
  try {
    put(await dmLayoutCmd.update(id, { collapsed }))
  } catch (e) {
    put(was)
    throw e
  }
}
