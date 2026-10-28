// Which profile card / full profile is open. One of each at a time; names and avatars open them.
export type CardTarget = { userId: string; serverId: string | null; anchor: DOMRect }
export type FullTarget = { userId: string; serverId: string | null }

export const profileUi: { card: CardTarget | null; full: FullTarget | null } = $state({ card: null, full: null })

/** Opens the quick card for `userId` next to `el`. */
export function openCard(userId: string, serverId: string | null, el: Element): void {
  profileUi.card = { userId, serverId, anchor: el.getBoundingClientRect() }
}

export function openFull(userId: string, serverId: string | null): void {
  profileUi.full = { userId, serverId }
}

export function closeCard(): void {
  profileUi.card = null
}

export function closeFull(): void {
  profileUi.full = null
}
