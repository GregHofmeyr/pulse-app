//! Global binds (Windows; on Linux the compositor binds the CLI, see ipc.rs). Low-level keyboard and
//! mouse hooks see every key and button system-wide and always pass it on: a bind toggles mute or
//! deafen and the key still works in the focused app, like Discord. Which press fires what is
//! decided by `bindmatch::Matcher`. Paused while a bind is being recorded. Windows can drop the
//! hooks silently; a watchdog notices (`hookwatch`) and reinstalls them.
//!
//! The hooks exist only while they're wanted: the user's switch is on AND something is bound
//! (`hookwatch::hooks_wanted`). Switched off, or with nothing bound (first run included), both hooks
//! are removed (not just ignored), the watchdog stands down and stops polling input, and at startup
//! they're never installed. Recording a bind doesn't need them: Settings records it from the
//! window's own key events. The hook thread (and the watchdog) still run, idle, so switching back
//! on, or adding the first bind, is instant.

#[cfg(windows)]
mod platform {
    use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};
    use std::sync::{LazyLock, Mutex, OnceLock, mpsc};
    use std::time::{Duration, Instant};

    use tauri::AppHandle;
    use windows::Win32::Foundation::{CloseHandle, HANDLE, LPARAM, LRESULT, WPARAM};
    use windows::Win32::Security::{
        GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::StationsAndDesktops::{
        CloseDesktop, DESKTOP_CONTROL_FLAGS, DESKTOP_READOBJECTS, OpenInputDesktop,
    };
    use windows::Win32::System::SystemInformation::GetTickCount;
    use windows::Win32::System::Threading::{
        GetCurrentProcess, GetCurrentThread, GetCurrentThreadId, OpenProcess, OpenProcessToken,
        PROCESS_QUERY_LIMITED_INFORMATION, SetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, GetLastInputInfo, LASTINPUTINFO, MAPVK_VK_TO_VSC_EX, MapVirtualKeyW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, HHOOK,
        KBDLLHOOKSTRUCT, LLKHF_EXTENDED, LLKHF_INJECTED, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT,
        PM_NOREMOVE, PeekMessageW, PostThreadMessageW, SetWindowsHookExW, UnhookWindowsHookEx,
        WH_KEYBOARD_LL, WH_MOUSE_LL, WM_APP, WM_KEYDOWN, WM_KEYUP, WM_MBUTTONDOWN, WM_MBUTTONUP,
        WM_SYSKEYDOWN, WM_SYSKEYUP, WM_XBUTTONDOWN, WM_XBUTTONUP,
    };

    use crate::bindmatch::{Matcher, key_code};
    use crate::hookwatch::{
        Check, Emit, Episode, Gate, HookStep, HookWatch, Hooks, LATE_INPUT_MS, MaxMeter,
        OWN_SLOW_US, Seen, Stall, StallLog, hook_step, hooks_wanted, overslept_ms, retry_wait,
        settled, switch_problem, tick_delay, watch_gate,
    };
    use crate::keybinds::{Action, Keybinds};

    /// Locked by the hook procedures once per event and briefly by apply/pause; nothing else.
    static MATCHER: LazyLock<Mutex<Matcher>> = LazyLock::new(Default::default);
    /// Fired actions go to a forwarder thread: the hook procedure must return at once, or Windows
    /// silently drops the hook (LowLevelHooksTimeout).
    static FIRED: OnceLock<mpsc::Sender<Action>> = OnceLock::new();
    /// Set if the hooks couldn't be installed, cleared when they are or when a change retries
    /// them; reported in Settings while hooks are wanted. (A missing hook thread isn't kept here:
    /// it's THREAD_ALIVE, so a retry can't wipe it.)
    static BROKEN: Mutex<Option<String>> = Mutex::new(None);
    /// The user's switch. Written by `install`/`set_enabled` (then a WM_SWITCH is posted), read by
    /// the hook thread when it handles a message: the switch as it is then, not as it was.
    static ENABLED: AtomicBool = AtomicBool::new(true);
    /// Whether anything is bound. Written by `install`/`apply` (then a WM_SWITCH is posted), read
    /// like ENABLED: hooks are wanted only when both are set.
    static HAS_BINDS: AtomicBool = AtomicBool::new(false);
    /// What the hook thread holds (`Hooks` as u8; it alone writes this). The watchdog only polls
    /// input while it's `In`, so never while hooks aren't wanted nor mid-reinstall.
    static HOOKS: AtomicU8 = AtomicU8::new(Hooks::None as u8);
    /// The hook thread, for posting to.
    static HOOK_THREAD: OnceLock<u32> = OnceLock::new();
    /// Failed attempts in the current reinstall (hook thread writes it; 0 outside one), so a
    /// reinstall that keeps failing shows in Settings.
    static REINSTALL_FAILURES: AtomicU32 = AtomicU32::new(0);
    /// The hook thread started and hasn't left its message loop. Nothing is posted to it
    /// otherwise, and while hooks are wanted Settings says so.
    static THREAD_ALIVE: AtomicBool = AtomicBool::new(false);
    /// `GetTickCount` when a hook last ran: the watchdog's evidence that they're still installed.
    static LAST_HOOK_TICK: AtomicU32 = AtomicU32::new(0);

    /// Posted to the hook thread to reinstall its hooks (hooks belong to the installing thread).
    const WM_REINSTALL: u32 = WM_APP + 1;
    /// Posted to the hook thread after ENABLED changed: install or remove the hooks to match.
    const WM_SWITCH: u32 = WM_APP + 2;
    /// The watchdog's timestamped ping (lParam = GetTickCount when posted).
    const WM_PING: u32 = WM_APP + 3;
    /// How often the watchdog looks.
    const WATCH_EVERY: Duration = Duration::from_secs(3);
    /// System input this much newer than the hooks' last event means they're gone; less is the
    /// race between an event reaching the hook and the watchdog reading the system tick. A hook
    /// thread stalled for longer than this reads as dead too (REINSTALL_GAP caps the cost).
    const DEAD_MARGIN_MS: u32 = 1000;
    /// At most one reinstall per this long; also the pause between failed reinstall attempts.
    const REINSTALL_GAP: Duration = Duration::from_secs(10);
    /// The reinstall pause is slept in slices this long, so it stops this soon after hooks stop
    /// being wanted (switched off, last bind removed).
    const RETRY_SLICE: Duration = Duration::from_millis(100);
    /// How long a settings change waits for the hook thread to catch up before replying.
    const SETTLE: Duration = Duration::from_millis(200);

    fn wanted() -> bool {
        hooks_wanted(
            ENABLED.load(Ordering::SeqCst),
            HAS_BINDS.load(Ordering::SeqCst),
        )
    }

    fn hooks() -> Hooks {
        Hooks::from_u8(HOOKS.load(Ordering::SeqCst))
    }

    /// Hook thread only.
    fn set_hooks(h: Hooks) {
        HOOKS.store(h as u8, Ordering::SeqCst);
    }

    /// The machinery as it is now, for the `hookwatch` decisions.
    fn with_seen<R>(f: impl FnOnce(&Seen) -> R) -> R {
        let broken = BROKEN.lock().unwrap_or_else(|e| e.into_inner()).clone();
        f(&Seen {
            wanted: wanted(),
            hooks: hooks(),
            thread_alive: THREAD_ALIVE.load(Ordering::SeqCst),
            install_failed: broken.as_deref(),
            reinstall_failures: REINSTALL_FAILURES.load(Ordering::SeqCst),
        })
    }

    /// Per watchdog tick (see `hookwatch::Stall`): input reaching our hook late, our procedure's
    /// own run time (µs), and the hook thread picking up a ping.
    static INPUT_LATE: MaxMeter = MaxMeter::new(LATE_INPUT_MS);
    static OWN_RUN: MaxMeter = MaxMeter::new(OWN_SLOW_US);
    static PING_LATE: MaxMeter = MaxMeter::new(LATE_INPUT_MS);

    /// Called first thing in each hook procedure with the event's own timestamp: atomics only,
    /// nothing that can block. An injected event's stamp is whatever the injecting tool chose, so
    /// it only counts as input seen.
    fn saw_input(event_ms: u32, injected: bool) {
        // SAFETY: GetTickCount has no preconditions.
        let now = unsafe { GetTickCount() };
        LAST_HOOK_TICK.store(now, Ordering::Relaxed);
        if !injected && let Some(late) = tick_delay(event_ms, now) {
            INPUT_LATE.note(late);
        }
    }

    /// Called last in each hook procedure, before passing the event on.
    fn ran_for(started: Instant) {
        OWN_RUN.note(u32::try_from(started.elapsed().as_micros()).unwrap_or(u32::MAX));
    }

    fn matcher() -> std::sync::MutexGuard<'static, Matcher> {
        MATCHER.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether Windows thinks a modifier is down (it tracks key-ups we never saw).
    fn still_down(code: &str) -> bool {
        let vk = match code {
            "ControlLeft" => 0xA2,
            "ControlRight" => 0xA3,
            "ShiftLeft" => 0xA0,
            "ShiftRight" => 0xA1,
            "AltLeft" => 0xA4,
            "AltRight" => 0xA5,
            "MetaLeft" => 0x5B,
            "MetaRight" => 0x5C,
            _ => return true,
        };
        (unsafe { GetAsyncKeyState(vk) }) < 0
    }

    fn event(code: &'static str, down: bool) {
        if !down {
            return matcher().up(code);
        }
        let fired = {
            let mut m = matcher();
            m.forget_released(still_down);
            m.down(code, Instant::now())
        };
        if let Some(tx) = FIRED.get() {
            for a in fired {
                let _ = tx.send(a);
            }
        }
    }

    /// Keys by physical position (scan code), so a layout switch doesn't move binds. Injected keys
    /// (gaming-keyboard software, remappers) count like real ones; injected text (VK_PACKET)
    /// doesn't.
    unsafe extern "system" fn keyboard(n: i32, w: WPARAM, l: LPARAM) -> LRESULT {
        if n >= 0 {
            let started = Instant::now();
            // SAFETY: for WH_KEYBOARD_LL with n >= 0, lParam points to a KBDLLHOOKSTRUCT.
            let k = unsafe { &*(l.0 as *const KBDLLHOOKSTRUCT) };
            saw_input(k.time, k.flags.0 & LLKHF_INJECTED.0 != 0);
            let ext = k.flags.0 & LLKHF_EXTENDED.0 != 0;
            // SAFETY: MapVirtualKeyW only reads its arguments.
            let to_scan = |vk| unsafe { MapVirtualKeyW(vk, MAPVK_VK_TO_VSC_EX) };
            if let Some(code) = key_code(k.vkCode, k.scanCode, ext, to_scan) {
                match w.0 as u32 {
                    WM_KEYDOWN | WM_SYSKEYDOWN => event(code, true),
                    WM_KEYUP | WM_SYSKEYUP => event(code, false),
                    _ => {}
                }
            }
            ran_for(started);
        }
        unsafe { CallNextHookEx(None, n, w, l) }
    }

    /// Wheel click and the side buttons; everything else (moves, left/right clicks) passes untouched.
    unsafe extern "system" fn mouse(n: i32, w: WPARAM, l: LPARAM) -> LRESULT {
        if n >= 0 {
            let started = Instant::now();
            // SAFETY: for WH_MOUSE_LL with n >= 0, lParam points to an MSLLHOOKSTRUCT.
            let m = unsafe { &*(l.0 as *const MSLLHOOKSTRUCT) };
            // every event, moves included: they update the system's last-input tick too
            saw_input(m.time, m.flags & LLMHF_INJECTED != 0);
            let msg = w.0 as u32;
            let button = match msg {
                WM_MBUTTONDOWN | WM_MBUTTONUP => Some("Mouse3"),
                WM_XBUTTONDOWN | WM_XBUTTONUP => match (m.mouseData >> 16) & 0xFFFF {
                    1 => Some("Mouse4"),
                    2 => Some("Mouse5"),
                    _ => None,
                },
                _ => None,
            };
            if let Some(code) = button {
                event(code, matches!(msg, WM_MBUTTONDOWN | WM_XBUTTONDOWN));
            }
            ran_for(started);
        }
        unsafe { CallNextHookEx(None, n, w, l) }
    }

    fn hook() -> Result<[HHOOK; 2], String> {
        // SAFETY: plain Win32 calls; the procedures are 'static functions.
        unsafe {
            let module = GetModuleHandleW(None).map_err(|e| e.to_string())?;
            let keys = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard), Some(module.into()), 0)
                .map_err(|e| e.to_string())?;
            match SetWindowsHookExW(WH_MOUSE_LL, Some(mouse), Some(module.into()), 0) {
                Ok(buttons) => Ok([keys, buttons]),
                Err(e) => {
                    let _ = UnhookWindowsHookEx(keys);
                    Err(e.to_string())
                }
            }
        }
    }

    fn unhook(hooks: [HHOOK; 2]) {
        for h in hooks {
            // Windows may already have removed it; nothing to do if so.
            // SAFETY: h came from SetWindowsHookExW on this thread and isn't used again.
            let _ = unsafe { UnhookWindowsHookEx(h) };
        }
    }

    /// Windows times a hook callback (LowLevelHooksTimeout) from the input event, waiting to be
    /// scheduled included, so a normal-priority hook thread on a machine a game is loading can
    /// overrun it and lose its hooks. TIME_CRITICAL is the top of the normal priority class (not
    /// real-time): ahead of any app's own threads, ABOVE_NORMAL isn't, as games often raise
    /// theirs. It costs nothing: the thread is blocked in GetMessageW but for a few microseconds
    /// per event.
    fn raise_priority() -> Result<(), String> {
        // SAFETY: GetCurrentThread's pseudo-handle is always valid on the calling thread.
        unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL) }
            .map_err(|e| e.to_string())
    }

    fn set_broken(problem: Option<String>) {
        *BROKEN.lock().unwrap_or_else(|e| e.into_inner()) = problem;
    }

    /// Marks the hook thread gone however it ends (a panic included). Windows removes a thread's
    /// hooks when it exits.
    struct Alive;

    impl Drop for Alive {
        fn drop(&mut self) {
            THREAD_ALIVE.store(false, Ordering::SeqCst);
            set_hooks(Hooks::None);
            tracing::warn!("keybinds: hook thread stopped; global binds are off until restart");
        }
    }

    /// Hooks are delivered through the installing thread's message loop, so they get their own
    /// thread that does nothing else (but add, remove or reinstall them when asked). It starts
    /// with no hooks and installs them only if they're wanted (switch on, something bound).
    fn run_hooks(ready: mpsc::Sender<u32>) {
        THREAD_ALIVE.store(true, Ordering::SeqCst);
        let _alive = Alive;
        if let Err(e) = raise_priority() {
            tracing::warn!(error = %e, "keybinds: hook thread priority couldn't be raised");
        }
        // Make sure this thread has a message queue before anyone posts to it.
        let mut msg = MSG::default();
        // SAFETY: msg is a valid MSG to write into.
        let _ = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE) };
        let mut hooks = step(None, false);
        // SAFETY: GetCurrentThreadId has no preconditions.
        let _ = ready.send(unsafe { GetCurrentThreadId() });
        loop {
            // SAFETY: msg is a valid MSG to write into.
            let got = unsafe { GetMessageW(&mut msg, None, 0, 0) }.0;
            if got == 0 || got == -1 {
                break;
            }
            let reinstall_asked = match msg.message {
                WM_REINSTALL => true,
                WM_SWITCH => false,
                WM_PING => {
                    // SAFETY: GetTickCount has no preconditions.
                    let now = unsafe { GetTickCount() };
                    if let Some(late) = tick_delay(msg.lParam.0 as u32, now) {
                        PING_LATE.note(late);
                    }
                    continue;
                }
                _ => continue,
            };
            hooks = step(hooks, reinstall_asked);
        }
        if let Some(h) = hooks {
            unhook(h);
        }
    }

    /// One message's worth of work on the hook thread: make the hooks match what's wanted, or
    /// reinstall them for the watchdog (`hookwatch::hook_step` decides which).
    fn step(hooks: Option<[HHOOK; 2]>, reinstall_asked: bool) -> Option<[HHOOK; 2]> {
        match hook_step(wanted(), hooks.is_some(), reinstall_asked) {
            HookStep::Nothing => hooks,
            HookStep::Unhook => {
                if let Some(h) = hooks {
                    unhook(h);
                }
                set_hooks(Hooks::None);
                tracing::info!("keybinds: switched off or nothing bound; hooks removed");
                None
            }
            HookStep::Install => match hook() {
                Ok(h) => {
                    // Fresh hooks: their tick starts now (before HOOKS, which the watchdog reads
                    // first), and keys released while there were none are forgotten.
                    // SAFETY: GetTickCount has no preconditions.
                    LAST_HOOK_TICK.store(unsafe { GetTickCount() }, Ordering::Relaxed);
                    matcher().forget_released(still_down);
                    set_broken(None);
                    set_hooks(Hooks::In);
                    tracing::info!("keybinds: hooks installed");
                    Some(h)
                }
                Err(e) => {
                    tracing::warn!(error = %e, "keybinds: hooks couldn't be installed");
                    set_broken(Some(format!(
                        "Keybinds couldn't start ({e}); change a bind, switch them off and on, or restart Pulse, to try again"
                    )));
                    None
                }
            },
            HookStep::Reinstall => reinstall(hooks),
        }
    }

    /// The watchdog's reinstall: unhook, then hook again until it works or hooks stop being
    /// wanted. Meanwhile HOOKS says `Retrying` (there are none), so the watchdog leaves input
    /// alone; it says `In` again only once new hooks really are.
    fn reinstall(hooks: Option<[HHOOK; 2]>) -> Option<[HHOOK; 2]> {
        set_hooks(Hooks::Retrying);
        if let Some(h) = hooks {
            unhook(h);
        }
        // LAST_HOOK_TICK is left alone: only a real event moving it tells the watchdog the
        // new hooks work.
        let mut failures = 0u32;
        REINSTALL_FAILURES.store(0, Ordering::SeqCst);
        let hooks = loop {
            match hook() {
                Ok(h) => break h,
                Err(e) => {
                    if failures == 0 {
                        tracing::warn!(error = %e, "keybinds: hooks couldn't be reinstalled; retrying");
                    } else {
                        tracing::debug!(error = %e, failures, "keybinds: hook reinstall failed again");
                    }
                    failures += 1;
                    REINSTALL_FAILURES.store(failures, Ordering::SeqCst);
                    // No hooks to service, so blocking this thread is harmless; the wait ends
                    // within a slice of hooks no longer being wanted.
                    if !retry_wait(REINSTALL_GAP, RETRY_SLICE, wanted, std::thread::sleep) {
                        REINSTALL_FAILURES.store(0, Ordering::SeqCst);
                        set_hooks(Hooks::None);
                        tracing::info!(
                            "keybinds: switched off or nothing bound; reinstall dropped"
                        );
                        return None;
                    }
                }
            }
        };
        REINSTALL_FAILURES.store(0, Ordering::SeqCst);
        // Unwanted while hook() ran (after the last check): don't leave them in, even briefly.
        if hook_step(wanted(), true, false) == HookStep::Unhook {
            unhook(hooks);
            set_hooks(Hooks::None);
            tracing::info!("keybinds: switched off or nothing bound; reinstall dropped");
            return None;
        }
        // Key-ups missed while the hooks were gone: drop modifiers Windows says are up (a
        // stale plain key is already treated as a fresh press, REPEAT_WINDOW).
        matcher().forget_released(still_down);
        set_hooks(Hooks::In);
        tracing::info!("keybinds: hooks reinstalled");
        Some(hooks)
    }

    fn token_elevated(process: HANDLE) -> Option<bool> {
        let mut token = HANDLE::default();
        // SAFETY: process is a valid process handle; token is written on success.
        unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }.ok()?;
        let mut elevation = TOKEN_ELEVATION::default();
        let mut len = 0u32;
        // SAFETY: the buffer is a TOKEN_ELEVATION of the size passed.
        let got = unsafe {
            GetTokenInformation(
                token,
                TokenElevation,
                Some(&mut elevation as *mut _ as *mut _),
                size_of::<TOKEN_ELEVATION>() as u32,
                &mut len,
            )
        };
        // SAFETY: token was opened above and isn't used again.
        let _ = unsafe { CloseHandle(token) };
        got.ok()?;
        Some(elevation.TokenIsElevated != 0)
    }

    /// Whether the hooks can see input right now. They can't while the secure desktop (UAC, lock
    /// screen) has the input, nor (UIPI) while a window more privileged than us is in front, yet
    /// the system's last-input tick still moves: a check then would read as a drop.
    fn hooks_can_see_input(self_elevated: bool) -> bool {
        // SAFETY: plain Win32 calls; every handle opened here is closed here.
        unsafe {
            match OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_READOBJECTS) {
                Ok(desk) => {
                    let _ = CloseDesktop(desk);
                }
                Err(_) => return false,
            }
            if self_elevated {
                return true;
            }
            let window = GetForegroundWindow();
            if window.is_invalid() {
                return false;
            }
            let mut pid = 0u32;
            if GetWindowThreadProcessId(window, Some(&mut pid)) == 0 || pid == 0 {
                return false;
            }
            let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
                return false;
            };
            let elevated = token_elevated(process);
            let _ = CloseHandle(process);
            elevated == Some(false)
        }
    }

    /// Asks the hook thread to do something. Callers log a failure (once per run of them).
    fn post(thread: u32, message: u32) -> Result<(), String> {
        // SAFETY: posting a thread message with no pointers in it.
        unsafe { PostThreadMessageW(thread, message, WPARAM(0), LPARAM(0)) }
            .map_err(|e| e.to_string())
    }

    /// A timestamped ping: how long the hook thread takes to pick it up is how long input would
    /// wait for it to get the CPU. Best effort.
    fn ping(thread: u32) {
        // SAFETY: GetTickCount has no preconditions; the message carries no pointers.
        unsafe {
            let now = GetTickCount();
            let _ = PostThreadMessageW(thread, WM_PING, WPARAM(0), LPARAM(now as isize));
        }
    }

    /// One watchdog tick's measurements, taken (and reset).
    fn take_stall(overslept: Duration) -> Stall {
        let (input_late_ms, late_events) = INPUT_LATE.take();
        Stall {
            input_late_ms,
            late_events,
            own_us: OWN_RUN.take().0,
            hook_thread_ms: PING_LATE.take().0,
            overslept_ms: overslept_ms(overslept),
        }
    }

    fn log_stall(e: Emit, since_reinstall: Option<Duration>) {
        // whether our hooks were just put back (first in the chain) or have sat behind others
        let since_reinstall_s = since_reinstall.map(|d| d.as_secs());
        match e {
            Emit::Nothing => {}
            Emit::Started(s) => tracing::warn!(
                input_late_ms = s.input_late_ms,
                late_events = s.late_events,
                hook_proc_us = s.own_us,
                hook_thread_ms = s.hook_thread_ms,
                overslept_ms = s.overslept_ms,
                ?since_reinstall_s,
                "keybinds: input stalled (the whole PC waits on low-level hooks)"
            ),
            Emit::Ongoing {
                since_last: s,
                lasted,
            } => tracing::warn!(
                input_late_ms = s.input_late_ms,
                late_events = s.late_events,
                hook_proc_us = s.own_us,
                hook_thread_ms = s.hook_thread_ms,
                overslept_ms = s.overslept_ms,
                lasted_s = lasted.as_secs(),
                "keybinds: input still stalling (worst in the last minute)"
            ),
            Emit::Ended {
                since_last: s,
                lasted,
            } => tracing::info!(
                input_late_ms = s.input_late_ms,
                late_events = s.late_events,
                hook_proc_us = s.own_us,
                hook_thread_ms = s.hook_thread_ms,
                overslept_ms = s.overslept_ms,
                lasted_s = lasted.as_secs(),
                "keybinds: input stall over"
            ),
        }
    }

    /// Low-level hooks get no notice when Windows drops them, so compare what they saw with what
    /// the system saw and have the hook thread reinstall them when they've gone quiet.
    fn watch_hooks(hook_thread: u32) {
        // SAFETY: GetCurrentProcess returns a pseudo-handle that needs no closing.
        let self_elevated = token_elevated(unsafe { GetCurrentProcess() }).unwrap_or(false);
        let mut watch = HookWatch::new(DEAD_MARGIN_MS, REINSTALL_GAP);
        let mut unreachable = Episode::default();
        let mut send = |message: u32| match post(hook_thread, message) {
            Ok(()) => {
                if unreachable.succeeded() {
                    tracing::info!("keybinds: hook thread reachable again");
                }
            }
            Err(e) => {
                if unreachable.failed() {
                    tracing::warn!(error = %e, message, "keybinds: couldn't reach the hook thread; retrying quietly");
                }
            }
        };
        let mut stalls = StallLog::default();
        let mut reinstalled_at: Option<Instant> = None;
        loop {
            let slept_from = Instant::now();
            std::thread::sleep(WATCH_EVERY);
            let overslept = slept_from.elapsed().saturating_sub(WATCH_EVERY);
            let emit = stalls.tick(take_stall(overslept), Instant::now());
            log_stall(emit, reinstalled_at.map(|t| t.elapsed()));
            ping(hook_thread);
            match with_seen(watch_gate) {
                Gate::Watch => {}
                Gate::Resend => {
                    // A lost (or late) WM_SWITCH: hooks and what's wanted disagree. The next
                    // hooks get a fresh watch either way.
                    watch.reset();
                    tracing::debug!("keybinds: hooks don't match the switch/binds; resending");
                    send(WM_SWITCH);
                    continue;
                }
                Gate::Idle => {
                    // not wanted, the install failed, or no hook thread (all shown in Settings
                    // as needed): nothing to watch, no input polled; the next hooks get a fresh
                    // watch
                    watch.reset();
                    continue;
                }
                // mid-reinstall: no hooks to check, and the backoff must survive it
                Gate::Wait => continue,
            }
            if !hooks_can_see_input(self_elevated) {
                continue;
            }
            let mut info = LASTINPUTINFO {
                cbSize: size_of::<LASTINPUTINFO>() as u32,
                dwTime: 0,
            };
            // SAFETY: info is a valid LASTINPUTINFO with cbSize set.
            if !unsafe { GetLastInputInfo(&mut info) }.as_bool() {
                continue;
            }
            let hook_ms = LAST_HOOK_TICK.load(Ordering::Relaxed);
            let behind_ms = info.dwTime.wrapping_sub(hook_ms);
            match watch.check(info.dwTime, hook_ms, Instant::now()) {
                Check::Fine => continue,
                Check::GaveUp => {
                    tracing::debug!(
                        behind_ms,
                        "keybinds: hooks still see no input after reinstalling; not retrying until they do"
                    );
                    continue;
                }
                Check::Reinstall => tracing::warn!(
                    behind_ms,
                    "keybinds: hooks stopped seeing input (dropped by Windows?); reinstalling"
                ),
            }
            reinstalled_at = Some(Instant::now());
            send(WM_REINSTALL);
        }
    }

    /// Returns the binds' problems (only "couldn't start", if the hooks failed). Hooks are
    /// installed only if wanted (`enabled` and something bound); the threads start regardless,
    /// idle, so switching on or binding something later works at once.
    pub fn install(
        app: &tauri::App,
        binds: &Keybinds,
        enabled: bool,
    ) -> tauri::Result<Vec<String>> {
        matcher().set_binds(binds);
        ENABLED.store(enabled, Ordering::SeqCst);
        HAS_BINDS.store(!binds.bound().is_empty(), Ordering::SeqCst);
        let (tx, rx) = mpsc::channel::<Action>();
        let _ = FIRED.set(tx);
        let handle = app.handle().clone();
        std::thread::Builder::new()
            .name("pulse-keys-fire".into())
            .spawn(move || {
                for a in rx {
                    crate::commands::hotkey(&handle, a.command());
                }
            })?;
        let (ready_tx, ready) = mpsc::channel();
        std::thread::Builder::new()
            .name("pulse-keys".into())
            .spawn(move || run_hooks(ready_tx))?;
        match ready.recv() {
            Ok(hook_thread) => {
                let _ = HOOK_THREAD.set(hook_thread);
                if !wanted() {
                    tracing::info!("keybinds: switched off or nothing bound; no hooks installed");
                }
                std::thread::Builder::new()
                    .name("pulse-keys-watch".into())
                    .spawn(move || watch_hooks(hook_thread))?;
            }
            // THREAD_ALIVE is false: Settings shows it once hooks are wanted
            Err(_) => tracing::warn!("keybinds: hook thread didn't start"),
        }
        Ok(problems())
    }

    /// Why binds aren't working, if they aren't. Not wanted (off, nothing bound) isn't a problem;
    /// hooks still in then is.
    pub fn problems() -> Vec<String> {
        problems_with(false)
    }

    /// `switch_on_its_way`: a WM_SWITCH was just delivered, so hooks still in while not wanted
    /// are the hook thread catching up, not a problem.
    fn problems_with(switch_on_its_way: bool) -> Vec<String> {
        with_seen(|s| switch_problem(s, switch_on_its_way))
            .into_iter()
            .collect()
    }

    /// New binds: the matcher takes them at once, and the hook thread is told, as the first
    /// bind installs the hooks and removing the last one removes them.
    pub fn apply(_app: &AppHandle, binds: &Keybinds) -> Vec<String> {
        matcher().set_binds(binds);
        HAS_BINDS.store(!binds.bound().is_empty(), Ordering::SeqCst);
        send_switch()
    }

    pub fn pause(_app: &AppHandle, paused: bool) -> Vec<String> {
        matcher().set_paused(paused);
        problems()
    }

    /// The master switch. The hook thread owns the hooks, so it's told to add or remove them.
    pub fn set_enabled(_app: &AppHandle, enabled: bool) -> Vec<String> {
        ENABLED.store(enabled, Ordering::SeqCst);
        send_switch()
    }

    /// After ENABLED or HAS_BINDS changed: tell the hook thread, which reads both when it gets to
    /// the message, so a burst of changes ends where the last one left them. If hooks are now
    /// wanted this retries a failed install, so the old failure is cleared. A post that fails is
    /// retried briefly here, and after that by the watchdog (`watch_gate`) until the hooks match.
    /// Then waits (up to SETTLE) for the hook thread, so the reply says what happened.
    fn send_switch() -> Vec<String> {
        if wanted() {
            set_broken(None);
        }
        let thread = match HOOK_THREAD.get() {
            Some(&t) if THREAD_ALIVE.load(Ordering::SeqCst) => t,
            // never started, or stopped: nothing to tell; shown if hooks are wanted
            _ => return problems(),
        };
        let mut delivered = false;
        for attempt in 0..3 {
            match post(thread, WM_SWITCH) {
                Ok(()) => {
                    delivered = true;
                    break;
                }
                Err(e) if attempt == 2 => {
                    tracing::warn!(error = %e, "keybinds: couldn't reach the hook thread; the watchdog will retry");
                }
                Err(_) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        if !delivered {
            // shown now if hooks are still in while not wanted; the watchdog keeps resending
            return problems();
        }
        // The hook thread is TIME_CRITICAL and does this in well under a millisecond, unless
        // it's mid-reinstall (not reading messages): then don't hold the caller.
        let until = Instant::now() + SETTLE;
        let caught_up = loop {
            if with_seen(settled) {
                break true;
            }
            if hooks() == Hooks::Retrying || Instant::now() >= until {
                break false;
            }
            std::thread::sleep(Duration::from_millis(2));
        };
        // Not caught up: on its way, so no "still in" warning now (if it really gets stuck, the
        // watchdog resends and Settings shows it when next opened).
        problems_with(!caught_up)
    }

    #[cfg(test)]
    mod tests {
        use windows::Win32::System::Threading::{
            GetCurrentThread, GetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL,
        };

        #[test]
        fn the_hook_thread_runs_time_critical() {
            std::thread::spawn(|| {
                super::raise_priority().unwrap();
                // SAFETY: GetCurrentThread's pseudo-handle is always valid on its own thread.
                let p = unsafe { GetThreadPriority(GetCurrentThread()) };
                assert_eq!(p, THREAD_PRIORITY_TIME_CRITICAL.0);
            })
            .join()
            .unwrap();
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use crate::keybinds::Keybinds;

    pub fn install(
        _app: &tauri::App,
        _binds: &Keybinds,
        _enabled: bool,
    ) -> tauri::Result<Vec<String>> {
        Ok(Vec::new())
    }

    pub fn problems() -> Vec<String> {
        Vec::new()
    }

    /// Stored and shown, but nothing to switch: the compositor binds the CLI (ipc.rs).
    pub fn set_enabled(_app: &tauri::AppHandle, _enabled: bool) -> Vec<String> {
        Vec::new()
    }

    pub fn apply(_app: &tauri::AppHandle, _binds: &Keybinds) -> Vec<String> {
        Vec::new()
    }

    pub fn pause(_app: &tauri::AppHandle, _paused: bool) -> Vec<String> {
        Vec::new()
    }
}

pub use platform::{apply, install, pause, problems, set_enabled};
