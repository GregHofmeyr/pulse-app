//! Offline outbox: messages are queued with a nonce, sent when possible, retried on reconnect,
//! and marked failed after 3 attempts. Never lost silently (spec §6.6).

use std::sync::{Arc, Mutex};

use pulse_protocol::ids::{ChannelId, MessageId};
use pulse_protocol::rest::SendMessageRequest;
use serde::Serialize;

use crate::api::{Api, ApiError};

const MAX_ATTEMPTS: u32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Sent,
    Failed,
}

pub type StatusSink = Arc<dyn Fn(&str, Status) + Send + Sync>;

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
            (self.status)(&i.nonce, Status::Failed);
        }
    }

    pub fn pending_len(&self) -> usize {
        self.items.lock().unwrap().len()
    }

    /// Try to send everything queued, in order. Network errors keep the item (up to 3 attempts);
    /// a server rejection (bad channel, too long…) fails it at once since retrying can't help.
    pub async fn flush(&self, api: &Api, token: &str) {
        let _one = self.flushing.lock().await;
        let queued: Vec<Item> = self.items.lock().unwrap().clone();
        for item in queued {
            let body = SendMessageRequest {
                content: item.content.clone(),
                reply_to_id: item.reply_to,
                nonce: Some(item.nonce.clone()),
            };
            let outcome = match api.send_message(token, item.channel, &body).await {
                Ok(_) => Some(Status::Sent),
                Err(ApiError::Network(_)) if item.attempts + 1 < MAX_ATTEMPTS => None,
                Err(_) => Some(Status::Failed),
            };
            let mut items = self.items.lock().unwrap();
            match outcome {
                Some(st) => {
                    items.retain(|i| i.nonce != item.nonce);
                    drop(items);
                    (self.status)(&item.nonce, st);
                }
                None => {
                    if let Some(i) = items.iter_mut().find(|i| i.nonce == item.nonce) {
                        i.attempts += 1;
                    }
                    // Still offline: keep order, don't hammer the rest.
                    break;
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
            Arc::new(move |nonce: &str, st: Status| {
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
        ob.flush(&Api::new("http://127.0.0.1:9"), &token).await;
        assert_eq!(ob.pending_len(), 1);

        // back online
        let api = Api::new(&format!("http://{}", app.addr));
        ob.flush(&api, &token).await;
        ob.flush(&api, &token).await; // a second flush must not resend
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
            ob.flush(&dead, "t").await;
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
        ob.flush(&Api::new(&format!("http://{}", app.addr)), &token)
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
        ob.flush(&Api::new(&format!("http://{}", app.addr)), &token)
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
}
