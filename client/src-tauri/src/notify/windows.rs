//! Windows: WinRT toasts. Each conversation's toast carries its channel id as the tag (group
//! "pulse"), so a new one replaces the last in Action Center instead of piling up.
//!
//! Built on the `windows` crate rather than tauri-winrt-notification: that wrapper has no tag or
//! group, and replacing needs both. It is the same handful of WinRT calls.

use super::toast::{KeepNewest, refusal, toast_xml};
use super::{Click, Failure, Notice, OnClick};
use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::TypedEventHandler;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
use windows::core::{HSTRING, IInspectable};

const GROUP: &str = "pulse";

pub struct Backend {
    app_id: HSTRING,
    on_click: OnClick,
    /// The newest toast per conversation: kept so its click handler lives while it's on screen
    /// (an unpackaged app only hears a click through the object it showed). For the newest
    /// [super::KEEP] conversations only, so a long session doesn't hold one for every chat.
    shown: KeepNewest<ToastNotification>,
}

impl Backend {
    pub fn new(app_id: String, on_click: OnClick) -> Self {
        Backend {
            app_id: HSTRING::from(app_id),
            on_click,
            shown: KeepNewest::default(),
        }
    }

    pub fn show(&mut self, n: &Notice) -> Result<(), Failure> {
        // A toast that won't build is this notice's problem; a notifier that won't show it is
        // the system's.
        let toast = self
            .build(n)
            .map_err(|e| Failure::BadNotice(e.to_string()))?;
        let notifier = ToastNotificationManager::CreateToastNotifierWithId(&self.app_id)
            .map_err(|e| Failure::Refused(e.to_string()))?;
        // Show() succeeds even when Windows then drops the toast (off for the app or the user,
        // or an AppUserModelID no shortcut carries, as with a copied exe): the notifier's setting
        // is the only sign of that. An unregistered id is expected to read as "disabled by
        // manifest" (not verified on a real machine). A setting that can't be read, or one newer
        // than we know, doesn't stop the toast: better shown than lost on a guess. If Windows
        // reads "enabled" and still drops it, there is nothing left to detect.
        if let Some(why) = notifier.Setting().ok().and_then(|s| refusal(s.0)) {
            return Err(Failure::Refused(why.into()));
        }
        notifier
            .Show(&toast)
            .map_err(|e| Failure::Refused(e.to_string()))?;
        self.shown.insert(n.channel_id.clone(), toast);
        Ok(())
    }

    fn build(&self, n: &Notice) -> windows::core::Result<ToastNotification> {
        let doc = XmlDocument::new()?;
        doc.LoadXml(&HSTRING::from(toast_xml(&n.title, &n.body)))?;
        let toast = ToastNotification::CreateToastNotification(&doc)?;
        toast.SetTag(&HSTRING::from(n.channel_id.as_str()))?;
        toast.SetGroup(&HSTRING::from(GROUP))?;
        let click = Click {
            channel_id: n.channel_id.clone(),
            thread_id: n.thread_id.clone(),
        };
        let on_click = self.on_click.clone();
        toast.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(
            move |_, _| {
                on_click(click.clone());
                Ok(())
            },
        ))?;
        Ok(toast)
    }
}
