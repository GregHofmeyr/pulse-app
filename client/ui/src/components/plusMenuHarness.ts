// Tests: the composer's + menu (#461). `plusItem('attach')` opens the menu and returns its Attach files row.
import { flushSync } from 'svelte'

export const plusButton = () => document.querySelector<HTMLButtonElement>('button[aria-label="More"]')

/** Opens the + menu (if it isn't) and returns the row `id` (attach, wheel, poll, plan, invite, later), or null. */
export function plusItem(id: string): HTMLButtonElement | null {
  if (!document.querySelector('[role=menu][aria-label="Add to your message"]')) {
    plusButton()?.click()
    flushSync()
  }
  return document.querySelector<HTMLButtonElement>(`[role=menu][aria-label="Add to your message"] [data-item="${id}"]`)
}

/** The + menu's rows as shown, `---` for a divider. */
export function plusRows(): string[] {
  const menu = document.querySelector('[role=menu][aria-label="Add to your message"]')
  return [...(menu?.children ?? [])].map((c) => (c.getAttribute('role') === 'separator' ? '---' : (c.textContent ?? '').trim()))
}
