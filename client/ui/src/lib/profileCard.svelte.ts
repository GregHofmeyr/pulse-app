// Which profile card / full profile is open. One of each at a time; names and avatars open them.
/** `opener` is the name or avatar that opened the card; `anchor` its rectangle then. */
export type CardTarget = { userId: string; serverId: string | null; anchor: DOMRect; opener: Element }
export type FullTarget = { userId: string; serverId: string | null }

/** `edit`: your own card asked for Settings → Profile (the shell opens it and clears this). */
export const profileUi: { card: CardTarget | null; full: FullTarget | null; edit: boolean } = $state({ card: null, full: null, edit: false })

/** Opens the quick card for `userId` next to `el`; the same name or avatar again closes it. */
export function openCard(userId: string, serverId: string | null, el: Element): void {
  const open = profileUi.card
  if (open && open.userId === userId && open.opener === el) profileUi.card = null
  else profileUi.card = { userId, serverId, anchor: el.getBoundingClientRect(), opener: el }
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

/** Edit profile, from your own card: the card and the full profile give way to Settings → Profile. */
export function editProfile(): void {
  profileUi.card = null
  profileUi.full = null
  profileUi.edit = true
}
