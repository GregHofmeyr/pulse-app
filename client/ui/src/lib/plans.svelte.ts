// Plan something's client side (#387). A plan is the same for everyone, so its `PlanChanged` event carries it whole
// and the reducer puts it in place (lib/state.ts); answers, edits and cancels answer with the plan too. Here: posting
// and changing plans, a forecast per plan (asked of the server once a session, only while the setting is on; a
// failed one can be asked again from the card, #399), the per-device "Plan something" setting, and the per-person
// "Plan reminders" one (#399, kept by the server).
import type { CreatePlanRequest } from './protocol/CreatePlanRequest'
import type { Message } from './protocol/Message'
import type { Plan } from './protocol/Plan'
import type { PlanForecast } from './protocol/PlanForecast'
import type { Rsvp } from './protocol/Rsvp'
import { applyEvent, setPlan } from './state'
import { app } from './store.svelte'
import { api, planCmd } from './tauri'

export { planPrefs, setPlanSomething } from './planPrefs.svelte'

const put = (channelId: string, id: string, plan: Plan): Plan => {
  app.state = setPlan(app.state, channelId, id, plan)
  return plan
}

/** Post a plan; it shows at once (its MessageCreated, when it comes, is the same). Rejects with the server's reason. */
export async function postPlan(channelId: string, request: CreatePlanRequest): Promise<Message> {
  const m = await planCmd.create(channelId, request)
  app.state = applyEvent(app.state, { t: 'MessageCreated', d: { message: m, nonce: null } }, Date.now())
  return m
}

/** Change your plan (every field). */
export const editPlan = async (m: Message, request: CreatePlanRequest): Promise<Plan> => {
  const plan = put(m.channel_id, m.id, await planCmd.edit(m.id, request))
  forecasts.delete(m.id) // a new place or day: ask again
  delete forecast.byId[m.id]
  return plan
}

/** Answer (none takes your answer back). */
export const rsvpPlan = async (m: Message, rsvp: Rsvp | null): Promise<Plan> => put(m.channel_id, m.id, await planCmd.rsvp(m.id, rsvp))

/** Cancel your plan. */
export const cancelPlan = async (m: Message): Promise<Plan> => put(m.channel_id, m.id, await planCmd.cancel(m.id))

/** Add to calendar: the saved path, or null when cancelled. */
export const saveIcs = (m: Message): Promise<string | null> => planCmd.saveIcs(m.id, m.plan?.title ?? 'plan')

/** Each plan's forecast as fetched this session (`null`: there's none), and the plans whose last ask failed (#399:
 *  the card offers Retry). */
export const forecast = $state({ byId: {} as Record<string, PlanForecast | null>, failed: {} as Record<string, true> })
const forecasts = new Map<string, Promise<void>>()

/** Ask the server for a plan's forecast once. A failure is kept (the card says so, with Retry) until it's asked
 *  again: by Retry, or by the next card that shows it. */
export function loadForecast(messageId: string): void {
  if (forecasts.has(messageId)) return
  delete forecast.failed[messageId]
  const go = planCmd.forecast(messageId).then(
    (f) => {
      if (forecasts.get(messageId) === go) forecast.byId[messageId] = f
    },
    () => {
      if (forecasts.get(messageId) !== go) return
      forecasts.delete(messageId)
      forecast.failed[messageId] = true
    },
  )
  forecasts.set(messageId, go)
}

/** Plan reminders (#399): whether plans' reminder lines ping you, on every device. Shows at once; put back (and
 *  rejects with the reason) when the server refuses. */
export async function setPlanReminders(on: boolean): Promise<void> {
  const was = app.state.planReminders
  app.state = { ...app.state, planReminders: on }
  try {
    await api.setPlanReminders(on)
  } catch (e) {
    app.state = { ...app.state, planReminders: was }
    throw e
  }
}

/** For tests, and on sign-out: forget every forecast. */
export function resetPlans(): void {
  forecasts.clear()
  forecast.byId = {}
  forecast.failed = {}
}
