//! Offline outbox: messages are queued with a nonce, sent when possible, retried on reconnect and
//! on a timer, and marked failed after 3 counted attempts. Never lost silently (spec §6.6).

use std::sync::{Arc, Mutex};

use pulse_protocol::ids::{ChannelId, MessageId};
use pulse_protocol::rest::{Message, SendMessageRequest};
use serde::Serialize;

use crate::api::{Api, ApiError};

const MAX_ATTEMPTS: u32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Sent,
    Failed,
}

/// (nonce, status, the confirmed message when sent)
pub type StatusSink = Arc<dyn Fn(&str, Status, Option<&Message>) + Send + Sync>;

/// Worth trying again later: we couldn't reach the server, or it was overloaded/restarting.
pub fn retryable(e: &ApiError) -> bool {
    matches!(e, ApiError::Network(_) | ApiError::Server(_))
}

#[derive(Clone)]
struct Item {
    nonce: String,
    channel: ChannelId,
    content: String,
    reply_to: Option<MessageId>,
    attempts: u32,
}

pub struct Outbox {
    items: Mutex<Vec<Item>>,
    /// One flush at a time, so an item is never sent twice concurrently.
    flushing: tokio::sync::Mutex<()>,
    status: StatusSink,
}

impl Outbox {
    pub fn new(status: StatusSink) -> Self {
        Self {
            items: Mutex::new(Vec::new()),
            flushing: tokio::sync::Mutex::new(()),
            status,
        }
    }

    pub fn enqueue(
        &self,
        nonce: String,
        channel: ChannelId,
        content: String,
        reply_to: Option<MessageId>,
    ) {
        self.items.lock().unwrap().push(Item {
            nonce,
            channel,
            content,
            reply_to,
            attempts: 0,
        });
    }

    /// Drop everything queued (logout / signed out): it must not be sent under the next account.
    pub fn clear(&self) {
        let dropped: Vec<Item> = std::mem::take(&mut *self.items.lock().unwrap());
        for i in dropped {
            (self.status)(&i.nonce, Status::Failed, None);
        }
    }

    pub fn pending_len(&self) -> usize {
        self.items.lock().unwrap().len()
    }

    /// Try to send everything queued, in order, stopping at the first retryable failure (keeps order).
    /// `count_attempts`: only reconnect/timer flushes count towards the 3 attempts — a burst of sends
    /// while offline must not fail the first queued message. Rejections fail at once.
    pub async fn flush(&self, api: &Api, token: &str, count_attempts: bool) {
        let _one = self.flushing.lock().await;
        let queued: Vec<Item> = self.items.lock().unwrap().clone();
        for item in queued {
            let body = SendMessageRequest {
                content: item.content.clone(),
                reply_to_id: item.reply_to,
                nonce: Some(item.nonce.clone()),
            };
            match api.send_message(token, item.channel, &body).await {
                Ok(m) => {
                    self.items.lock().unwrap().retain(|i| i.nonce != item.nonce);
                    (self.status)(&item.nonce, Status::Sent, Some(&m));
                }
                Err(e) if retryable(&e) => {
                    let failed_now = {
                        let mut items = self.items.lock().unwrap();
                        let gave_up = match items.iter_mut().find(|i| i.nonce == item.nonce) {
                            Some(i) if count_attempts => {
                                i.attempts += 1;
                                i.attempts >= MAX_ATTEMPTS
                            }
                            _ => false,
                        };
                        if gave_up {
                            items.retain(|i| i.nonce != item.nonce);
                        }
                        gave_up
                    };
                    if failed_now {
                        (self.status)(&item.nonce, Status::Failed, None);
                    }
                    break; // still offline: don't hammer the rest
                }
                Err(_) => {
                    self.items.lock().unwrap().retain(|i| i.nonce != item.nonce);
                    (self.status)(&item.nonce, Status::Failed, None);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pulse_server::testing;
    use std::sync::{Arc, Mutex};

    type Seen = Arc<Mutex<Vec<(String, Status)>>>;

    fn sink() -> (StatusSink, Seen) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        (
            Arc::new(move |nonce: &str, st: Status, _m: Option<&Message>| {
                s.lock().unwrap().push((nonce.to_string(), st))
            }),
            seen,
        )
    }

    #[tokio::test]
    async fn outbox_flushes_after_reconnect() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let s = testing::create_server(&app, &token, "Main").await;
        let g = testing::general(&app, &token, s.id).await;
        let (status, _) = sink();
        let ob = Outbox::new(status);
        ob.enqueue("n1".into(), g.id, "queued while offline".into(), None);

        // "offline": nothing is listening here
        ob.flush(&Api::new("http://127.0.0.1:9"), &token, true)
            .await;
        assert_eq!(ob.pending_len(), 1);

        // back online
        let api = Api::new(&format!("http://{}", app.addr));
        ob.flush(&api, &token, true).await;
        ob.flush(&api, &token, true).await; // a second flush must not resend
        assert_eq!(ob.pending_len(), 0);
        let msgs = api.messages(&token, g.id, None).await.unwrap();
        assert_eq!(
            msgs.iter()
                .filter(|m| m.content == "queued while offline")
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn outbox_marks_failed_after_three_attempts() {
        let (status, seen) = sink();
        let ob = Outbox::new(status);
        ob.enqueue(
            "n1".into(),
            pulse_protocol::ids::ChannelId::new(),
            "x".into(),
            None,
        );
        let dead = Api::new("http://127.0.0.1:9");
        for _ in 0..3 {
            ob.flush(&dead, "t", true).await;
        }
        assert!(
            seen.lock()
                .unwrap()
                .contains(&("n1".to_string(), Status::Failed))
        );
        assert_eq!(
            ob.pending_len(),
            0,
            "failed items leave the queue (UI offers retry)"
        );
    }

    #[tokio::test]
    async fn rejected_by_server_fails_immediately() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let (status, seen) = sink();
        let ob = Outbox::new(status);
        // unknown channel → 404: retrying won't help
        ob.enqueue(
            "n1".into(),
            pulse_protocol::ids::ChannelId::new(),
            "x".into(),
            None,
        );
        ob.flush(&Api::new(&format!("http://{}", app.addr)), &token, true)
            .await;
        assert_eq!(
            seen.lock().unwrap().as_slice(),
            &[("n1".to_string(), Status::Failed)]
        );
    }

    #[tokio::test]
    async fn nonce_echoed_in_message_created() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let s = testing::create_server(&app, &token, "Main").await;
        let g = testing::general(&app, &token, s.id).await;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let _gw =
            crate::gateway::GatewayHandle::spawn(format!("http://{}", app.addr), token.clone(), tx);
        while !matches!(
            rx.recv().await,
            Some(crate::gateway::GatewayUpdate::Ready(_))
        ) {}
        let (status, _) = sink();
        let ob = Outbox::new(status);
        ob.enqueue("my-nonce".into(), g.id, "hello".into(), None);
        ob.flush(&Api::new(&format!("http://{}", app.addr)), &token, true)
            .await;
        loop {
            if let Some(crate::gateway::GatewayUpdate::Event(
                pulse_protocol::gateway::Event::MessageCreated { nonce, .. },
            )) = rx.recv().await
            {
                assert_eq!(nonce.as_deref(), Some("my-nonce"));
                break;
            }
        }
    }

    #[tokio::test]
    async fn clear_fails_everything_queued() {
        let (status, seen) = sink();
        let ob = Outbox::new(status);
        ob.enqueue(
            "a".into(),
            pulse_protocol::ids::ChannelId::new(),
            "x".into(),
            None,
        );
        ob.enqueue(
            "b".into(),
            pulse_protocol::ids::ChannelId::new(),
            "y".into(),
            None,
        );
        ob.clear();
        assert_eq!(ob.pending_len(), 0);
        let seen = seen.lock().unwrap();
        assert!(
            seen.contains(&("a".to_string(), Status::Failed))
                && seen.contains(&("b".to_string(), Status::Failed))
        );
    }

    /// I4: sends while offline must not burn the first message's attempts.
    #[tokio::test]
    async fn sends_while_offline_do_not_fail_queued_messages() {
        let (status, seen) = sink();
        let ob = Outbox::new(status);
        ob.enqueue(
            "n1".into(),
            pulse_protocol::ids::ChannelId::new(),
            "x".into(),
            None,
        );
        let dead = Api::new("http://127.0.0.1:9");
        for _ in 0..5 {
            ob.flush(&dead, "t", false).await;
        }
        assert_eq!(ob.pending_len(), 1);
        assert!(seen.lock().unwrap().is_empty());
    }

    /// I3: the confirmed Message travels with "sent", so the UI can show it without the gateway.
    #[tokio::test]
    async fn sent_status_carries_the_message() {
        let app = testing::spawn().await;
        let (_, token) = testing::register(&app, "alex").await;
        let s = testing::create_server(&app, &token, "Main").await;
        let g = testing::general(&app, &token, s.id).await;
        let got: Arc<Mutex<Option<Message>>> = Arc::default();
        let g2 = got.clone();
        let ob = Outbox::new(Arc::new(
            move |_n: &str, st: Status, m: Option<&Message>| {
                if st == Status::Sent {
                    *g2.lock().unwrap() = m.cloned();
                }
            },
        ));
        ob.enqueue("n1".into(), g.id, "hello".into(), None);
        ob.flush(&Api::new(&format!("http://{}", app.addr)), &token, false)
            .await;
        assert_eq!(
            got.lock().unwrap().as_ref().map(|m| m.content.as_str()),
            Some("hello")
        );
    }

    #[test]
    fn server_trouble_is_retryable_rejections_are_not() {
        assert!(retryable(&ApiError::Network("x".into())));
        assert!(retryable(&ApiError::Server("502".into())));
        assert!(!retryable(&ApiError::Rejected("too long".into())));
        assert!(!retryable(&ApiError::Unauthorized));
    }
}
