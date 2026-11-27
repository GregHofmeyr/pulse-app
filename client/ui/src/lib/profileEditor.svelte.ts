// The profile editor (#727): which section is open, and the working copy every section edits. Everything, uploads and
// the badge order included, is staged here until Save sends it; the store isn't touched before then, so leaving
// without saving leaves you exactly as you were. The draft logic is lifted from the old ProfileSettings page (#467, retired in #727) as it was.
import { setProfile } from './state'
import { app, loadProfile } from './store.svelte'
import { api, errorText } from './tauri'
import { typingTextProblem } from './typingLine'
import { BIO_LINE_BREAKS_MAX, BIO_MAX, charCount, draftFrom, editsSince, isDirty, nameProblem, normalBio, PRONOUNS_MAX, toRequest, type Draft } from './profileDraft'
import { defaultShowcase } from './badges.svelte'
import type { CropRect } from './crop'
import type { Profile } from './protocol/Profile'

export type EditorSection = 'looks' | 'badges' | 'stats' | 'typing' | 'appearance'
type Kind = 'avatar' | 'banner'

/** Whether the editor is open and on which section. Shell mounts the editor from it. */
export const profileEditor = $state<{ open: boolean; section: EditorSection }>({ open: false, section: 'looks' })

export function openProfileEditor(section: EditorSection = 'looks'): void {
  profileEditor.section = section
  profileEditor.open = true
}

export function closeProfileEditor(): void {
  profileEditor.open = false
}


export class ProfileEditorState {
  profile = $state.raw<Profile>(undefined as never)
  draft = $state<Draft>(undefined as never)
  saved = $state.raw<Draft>(undefined as never)
  /** The card's badges (#467): up to SHOWCASE_MAX held badges in order, or `auto` (the rarest five, which follows later unlocks). */
  showcase = $state<string[] | 'auto'>('auto')
  savedShowcase = $state.raw<string[] | 'auto'>('auto')
  uploading = $state<Record<Kind, boolean>>({ avatar: false, banner: false })
  uploadErrors = $state<Record<Kind, string | null>>({ avatar: null, banner: null })
  saving = $state(false)
  error = $state('')
  /** What the last successful Save left on top of the answer (edits made while it was on its way); null before any. */
  lastSave = $state.raw<{ kept: Partial<Draft> } | null>(null)
  private gone = false
  /** Bumped per kind by each upload, Remove and Reset: an upload that lands after a newer one of these is dropped. */
  private gen: Record<Kind, number> = { avatar: 0, banner: 0 }

  constructor(profile: Profile) {
    this.take(profile)
  }

  /** Why an upload failed (the latest), or ''. */
  get uploadError(): string {
    return this.uploadErrors.avatar ?? this.uploadErrors.banner ?? ''
  }

  get profileChanged(): boolean {
    return isDirty(this.draft, this.saved)
  }
  get showcaseChanged(): boolean {
    const [a, b] = [this.showcase, this.savedShowcase]
    return a === 'auto' || b === 'auto' ? a !== b : a.join() !== b.join()
  }
  /** Unsaved changes. */
  get dirty(): boolean {
    return this.profileChanged || this.showcaseChanged
  }
  /** An upload still in flight: leaving would lose it. */
  get busy(): boolean {
    return this.uploading.avatar || this.uploading.banner
  }
  get invalid(): boolean {
    const d = this.draft
    const bio = normalBio(d.bio)
    return charCount(bio) > BIO_MAX || (bio.match(/\n/g)?.length ?? 0) > BIO_LINE_BREAKS_MAX || charCount(d.pronouns.trim()) > PRONOUNS_MAX || !!nameProblem(d.displayName) || !!typingTextProblem(d.typingText)
  }

  private take(p: Profile) {
    this.profile = p
    this.saved = draftFrom(p)
    this.draft = draftFrom(p)
    const mine = new Set(p.badges.map((b) => b.key))
    this.savedShowcase = p.showcase_chosen ? (p.showcase ?? []).filter((k) => mine.has(k)) : 'auto'
    this.showcase = this.savedShowcase === 'auto' ? 'auto' : [...this.savedShowcase]
  }

  /** The badges as the card shows them now. */
  get shown(): string[] {
    return this.showcase === 'auto' ? defaultShowcase(this.profile.badges) : this.showcase
  }

  /** Back to what was last saved. */
  reset(): void {
    this.gen.avatar++
    this.gen.banner++
    this.uploading = { avatar: false, banner: false }
    this.take(this.profile)
    this.uploadErrors = { avatar: null, banner: null }
    this.error = ''
  }

  /** The editor is going away: answers still on their way are dropped. */
  dispose(): void {
    this.gone = true
  }

  /** Upload the cropped picture and stage it. A refusal leaves the draft as it was. */
  async upload(kind: Kind, file: File, crop: CropRect): Promise<void> {
    const g = ++this.gen[kind]
    this.uploadErrors[kind] = null
    this.uploading[kind] = true
    try {
      const bytes = new Uint8Array(await file.arrayBuffer())
      const info = await (kind === 'avatar' ? api.uploadAvatar(bytes, crop) : api.uploadBanner(bytes, crop))
      if (this.gone || this.gen[kind] !== g) return // Reset or Remove since: dropped
      if (kind === 'avatar') {
        this.draft.avatarHash = info.hash
        this.draft.avatarStill = info.still_hash
      } else {
        this.draft.bannerHash = info.hash
        this.draft.bannerStill = info.still_hash
      }
    } catch (e) {
      if (!this.gone && this.gen[kind] === g) this.uploadErrors[kind] = errorText(e)
    } finally {
      if (!this.gone && this.gen[kind] === g) this.uploading[kind] = false
    }
  }

  /** Clears the picture, and drops an upload of it still in flight. */
  remove(kind: Kind): void {
    this.gen[kind]++
    this.uploading[kind] = false
    this.uploadErrors[kind] = null
    if (kind === 'avatar') this.draft.avatarHash = this.draft.avatarStill = null
    else this.draft.bannerHash = this.draft.bannerStill = null
  }

  /** Sends the draft (when it changed) and the card's badges (when they did). Resolves when answered; a failure
   *  is in `error` and keeps the draft. Does nothing when there is nothing to send, or it can't be sent now. */
  save(): Promise<void> {
    if (this.saving || !this.dirty || this.invalid || this.busy) return Promise.resolve()
    return this.send($state.snapshot(this.draft), this.profileChanged, this.showcaseChanged ? (this.showcase === 'auto' ? [] : [...this.showcase]) : null)
  }

  /** Sends `sent` (when `withProfile`) and the card's badges (`keys`, `[]` for the rarest five; null when unchanged);
   *  edits made while it is on its way are kept on top of the answer. */
  private async send(sent: Draft, withProfile: boolean, keys: string[] | null): Promise<void> {
    this.saving = true
    this.error = ''
    const sentShowcase = $state.snapshot(this.showcase)
    try {
      if (keys) await api.setBadgeShowcase(keys)
      const answer = withProfile ? await api.updateProfile(toRequest(sent)) : await loadProfile(app.state.me!.id)
      // The cache gets the answer even if the editor has gone meanwhile: the save went through.
      app.state = setProfile(app.state, answer)
      if (this.gone) return
      const edits = editsSince(sent, $state.snapshot(this.draft))
      const nowShowcase = $state.snapshot(this.showcase)
      this.take(answer)
      // a badge reorder made meanwhile stays too
      if (JSON.stringify(nowShowcase) !== JSON.stringify(sentShowcase)) this.showcase = nowShowcase
      Object.assign(this.draft, edits)
      this.lastSave = { kept: edits }
    } catch (e) {
      if (!this.gone) this.error = errorText(e)
    } finally {
      if (!this.gone) this.saving = false
    }
  }
}
