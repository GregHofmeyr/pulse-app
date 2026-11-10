//! Soundboard end to end through the client core's `Api`: upload an Ogg Opus file, add it to a
//! server, list, favourite, edit, play into a call (the server relays; playback itself is the
//! Tauri commands' job) and delete. The Ogg is hand-built so the test never needs ffmpeg.

use pulse_client::api::{Api, ApiError};
use pulse_protocol::rest::{ChannelKind, CreateSoundRequest, UpdateSoundRequest};
use pulse_server::testing;

/// Ogg's CRC-32 (poly 0x04c11db7, no reflection, no xor), for building valid pages.
fn crc(data: &[u8]) -> u32 {
    let mut crc = 0u32;
    for &b in data {
        crc ^= u32::from(b) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ 0x04c1_1db7
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn page(header_type: u8, granule: i64, seq: u32, packets: &[&[u8]]) -> Vec<u8> {
    let mut lacing = Vec::new();
    for p in packets {
        let mut n = p.len();
        while n >= 255 {
            lacing.push(255);
            n -= 255;
        }
        lacing.push(n as u8);
    }
    let mut out = b"OggS".to_vec();
    out.push(0);
    out.push(header_type);
    out.extend_from_slice(&granule.to_le_bytes());
    out.extend_from_slice(&7u32.to_le_bytes());
    out.extend_from_slice(&seq.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.push(lacing.len() as u8);
    out.extend_from_slice(&lacing);
    for p in packets {
        out.extend_from_slice(p);
    }
    let c = crc(&out);
    out[22..26].copy_from_slice(&c.to_le_bytes());
    out
}

/// A valid Ogg Opus file of `ms` milliseconds (the server never decodes, so the audio packets
/// are filler).
fn ogg_opus(ms: u32) -> Vec<u8> {
    let pre_skip = 312u16;
    let mut head = b"OpusHead".to_vec();
    head.extend_from_slice(&[1, 1]);
    head.extend_from_slice(&pre_skip.to_le_bytes());
    head.extend_from_slice(&48_000u32.to_le_bytes());
    head.extend_from_slice(&[0, 0, 0]);
    let mut tags = b"OpusTags".to_vec();
    tags.extend_from_slice(&0u32.to_le_bytes());
    tags.extend_from_slice(&0u32.to_le_bytes());
    let mut out = page(2, 0, 0, &[&head]);
    out.extend(page(0, 0, 1, &[&tags]));
    let audio = [[1u8, 2, 3, 4], [5, 6, 7, 8], [9, 10, 11, 12]];
    let packets: Vec<&[u8]> = audio.iter().map(|p| &p[..]).collect();
    out.extend(page(
        4,
        i64::from(ms) * 48 + i64::from(pre_skip),
        2,
        &packets,
    ));
    out
}

#[tokio::test]
async fn sounds_end_to_end() {
    let app = testing::spawn().await;
    let api = Api::new(&format!("http://{}", app.addr));
    let (alex_id, alex) = testing::register(&app, "alex").await;
    let srv = api.create_server(&alex, "Main").await.unwrap();

    let media = api.upload_sound_media(&alex, ogg_opus(1000)).await.unwrap();
    assert!(!media.hash.is_empty());
    let sound = api
        .create_sound(
            &alex,
            srv.id,
            &CreateSoundRequest {
                name: "airhorn".into(),
                emoji: Some("📯".into()),
                emoji_id: None,
                media_hash: media.hash.clone(),
                volume: 120,
            },
        )
        .await
        .unwrap();
    assert_eq!((sound.name.as_str(), sound.volume), ("airhorn", 120));
    assert_eq!(
        api.list_sounds(&alex, srv.id).await.unwrap(),
        std::slice::from_ref(&sound)
    );

    // edits: rename + volume, then clear the emoji
    let edited = api
        .update_sound(
            &alex,
            &sound.id,
            &UpdateSoundRequest {
                name: Some("horn".into()),
                volume: Some(80),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!((edited.name.as_str(), edited.volume), ("horn", 80));
    assert_eq!(
        edited.emoji.as_deref(),
        Some("📯"),
        "absent stays as it was"
    );
    let cleared = api
        .update_sound(
            &alex,
            &sound.id,
            &UpdateSoundRequest {
                emoji: Some(None),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(cleared.emoji, None);

    api.set_sound_favourite(&alex, &sound.id, true)
        .await
        .unwrap();
    api.set_sound_favourite(&alex, &sound.id, false)
        .await
        .unwrap();

    // join sounds (#121): set, then clear, as the member list shows
    let join_sound = |members: Vec<pulse_protocol::rest::Member>| {
        members
            .into_iter()
            .find(|m| m.user.username == "alex")
            .unwrap()
            .join_sound
    };
    api.set_join_sound(&alex, srv.id, Some(&sound.id))
        .await
        .unwrap();
    assert_eq!(
        join_sound(api.members(&alex, srv.id).await.unwrap()),
        Some(sound.id.clone())
    );
    let err = api
        .set_join_sound(&alex, srv.id, Some("no-such-sound"))
        .await
        .unwrap_err();
    assert!(matches!(err, ApiError::Rejected(_)), "{err:?}");
    api.set_join_sound(&alex, srv.id, None).await.unwrap();
    assert_eq!(join_sound(api.members(&alex, srv.id).await.unwrap()), None);

    // not in a call: the server says so
    let err = api.play_sound_to_call(&alex, &sound.id).await.unwrap_err();
    assert!(matches!(err, ApiError::Rejected(_)), "{err:?}");

    // in the Lounge: it plays
    let chans = api.channels(&alex, srv.id).await.unwrap();
    let lounge = chans
        .iter()
        .find(|c| c.kind == ChannelKind::Voice)
        .expect("a voice channel");
    app.state().voice.join(lounge.id, alex_id, "PA_a");
    api.play_sound_to_call(&alex, &sound.id).await.unwrap();

    api.delete_sound(&alex, &sound.id).await.unwrap();
    assert!(api.list_sounds(&alex, srv.id).await.unwrap().is_empty());
}
