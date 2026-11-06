// Which profile card / full profile is open. One of each at a time; names and avatars open them.
/** `opener` is the name or avatar that opened the card; `anchor` its rectangle then. */
export type CardTarget = { userId: string; serverId: string | null; anchor: DOMRect; opener: Element }
export type FullTarget = { userId: string; serverId: string | null }

/** A nickname being changed (#48): someone's in a server, or the private one only you see. */
export type RenameTarget = { kind: 'server'; userId: string; serverId: string } | { kind: 'private'; userId: string }

/** `edit`: your own card asked for Settings → Profile; `settings`: for Settings (#168). The shell opens it and clears
 *  the flag. `rename`: the nickname dialog (the shell shows it, above the full profile). */
export const profileUi: { card: CardTarget | null; full: FullTarget | null; edit: boolean; settings: boolean; rename: RenameTarget | null } = $state({ card: null, full: null, edit: false, settings: false, rename: null })

/** Asks for a nickname. The quick card gives way first (a click in the dialog would close it). */
export function openRename(target: RenameTarget): void {
  profileUi.card = null
  profileUi.rename = target
}

export function closeRename(): void {
  profileUi.rename = null
}

/** Opens the quick card for `userId` next to `el`; the same name or avatar again closes it. Inside a
 *  `[data-card-anchor]` (a member row, #182) it sits beside that instead, so it never covers the row's avatar. */
export function openCard(userId: string, serverId: string | null, el: Element): void {
  const open = profileUi.card
  if (open && open.userId === userId && open.opener === el) profileUi.card = null
  else profileUi.card = { userId, serverId, anchor: (el.closest('[data-card-anchor]') ?? el).getBoundingClientRect(), opener: el }
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

/** Settings, from your own card (#168): the card and the full profile give way to Settings. */
export function openCardSettings(): void {
  profileUi.card = null
  profileUi.full = null
  profileUi.settings = true
}
