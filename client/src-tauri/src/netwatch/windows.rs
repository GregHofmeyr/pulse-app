//! Windows wake-ups: `NotifyUnicastIpAddressChange` and `NotifyIpInterfaceChange` (IP Helper), and
//! `PowerRegisterSuspendResumeNotification` for resume from sleep. All three call back on system threads; each
//! callback only sends a [`Wake`]. Not filtered by adapter here: a Hyper-V/WSL vEthernet can also be the real NIC
//! (an external switch), and the route check after the debounce already ignores adapters that don't carry it.
//! Registrations last for the app's life (their context is leaked on purpose, so it outlives every callback).

use std::ffi::c_void;
use std::sync::mpsc::Sender;

use super::Wake;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::NetworkManagement::IpHelper::{
    MIB_IPINTERFACE_ROW, MIB_NOTIFICATION_TYPE, MIB_UNICASTIPADDRESS_ROW, NotifyIpInterfaceChange,
    NotifyUnicastIpAddressChange,
};
use windows::Win32::Networking::WinSock::AF_UNSPEC;
use windows::Win32::System::Power::{
    DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS, PowerRegisterSuspendResumeNotification,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DEVICE_NOTIFY_CALLBACK, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND,
};

/// Starts the sources; true if network notifications are live (otherwise the watcher polls).
pub fn start(tx: Sender<Wake>) -> bool {
    let ctx: &'static Sender<Wake> = Box::leak(Box::new(tx));
    let ctx_ptr = ctx as *const Sender<Wake> as *const c_void;
    // SAFETY: the callbacks only read `ctx`, which is 'static; the handles are kept for the app's life.
    let (addr, iface) = unsafe {
        let mut h = HANDLE::default();
        let addr =
            NotifyUnicastIpAddressChange(AF_UNSPEC, Some(on_addr), Some(ctx_ptr), false, &mut h);
        let mut h = HANDLE::default();
        let iface =
            NotifyIpInterfaceChange(AF_UNSPEC, Some(on_iface), Some(ctx_ptr), false, &mut h);
        (addr, iface)
    };
    if !addr.is_ok() {
        tracing::warn!(code = addr.0, "netwatch: no address notifications");
    }
    if !iface.is_ok() {
        tracing::warn!(code = iface.0, "netwatch: no interface notifications");
    }
    let params: &'static mut DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS =
        Box::leak(Box::new(DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
            Callback: Some(on_power),
            Context: ctx_ptr as *mut c_void,
        }));
    let mut reg: *mut c_void = std::ptr::null_mut();
    // SAFETY: with DEVICE_NOTIFY_CALLBACK the recipient is a DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS, leaked above.
    let power = unsafe {
        PowerRegisterSuspendResumeNotification(
            DEVICE_NOTIFY_CALLBACK,
            HANDLE(params as *mut DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS as *mut c_void),
            &mut reg,
        )
    };
    if !power.is_ok() {
        tracing::info!(code = power.0, "netwatch: no resume notifications");
    }
    addr.is_ok() || iface.is_ok()
}

/// SAFETY (all callbacks): `ctx` is the leaked `Sender<Wake>` from `start`.
unsafe fn send(ctx: *const c_void, w: Wake) {
    if let Some(tx) = unsafe { (ctx as *const Sender<Wake>).as_ref() } {
        let _ = tx.send(w);
    }
}

unsafe extern "system" fn on_addr(
    ctx: *const c_void,
    _row: *const MIB_UNICASTIPADDRESS_ROW,
    _kind: MIB_NOTIFICATION_TYPE,
) {
    unsafe { send(ctx, Wake::Network) }
}

unsafe extern "system" fn on_iface(
    ctx: *const c_void,
    _row: *const MIB_IPINTERFACE_ROW,
    _kind: MIB_NOTIFICATION_TYPE,
) {
    unsafe { send(ctx, Wake::Network) }
}

unsafe extern "system" fn on_power(ctx: *const c_void, kind: u32, _setting: *const c_void) -> u32 {
    if kind == PBT_APMRESUMEAUTOMATIC || kind == PBT_APMRESUMESUSPEND {
        unsafe { send(ctx, Wake::Resumed) }
    }
    0
}
