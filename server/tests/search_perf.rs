//! Search speed on a friend-group-sized server: ~50k messages over five channels. Ignored by
//! default (it takes a while to fill); run with
//! `cargo test -p pulse-server --test search_perf -- --ignored --nocapture`.
mod common;

use std::time::{Duration, Instant};

use common::*;
use pulse_protocol::rest::SearchResults;

const MESSAGES: usize = 50_000;
const BUDGET: Duration = Duration::from_millis(50);

/// A deterministic word list (syllable pairs) and a cheap generator, so runs are comparable.
fn vocabulary() -> Vec<String> {
    let syl = [
        "ba", "ko", "ri", "te", "mu", "sa", "lo", "pe", "na", "di", "ga", "fu", "zo", "chi", "ve",
        "ya", "wu", "ha", "ni", "ro", "ke", "pi", "su", "to", "me", "la", "jo", "bi", "du", "fe",
        "go", "hu", "ji", "ku", "le", "mo", "nu", "po", "ra", "si", "tu", "va", "we", "xi",
    ];
    let mut v = Vec::new();
    for a in syl {
        for b in syl {
            v.push(format!("{a}{b}"));
        }
    }
    v
}

#[tokio::test]
#[ignore = "fills 50k messages; run by hand (see the module doc)"]
async fn typical_queries_take_under_50ms() {
    let app = spawn().await;
    let (alex_id, alex) = register(&app, "alex").await;
    let (sam_id, _) = register(&app, "sam").await;
    let s = create_server(&app, &alex, "Main").await;
    let mut channels = vec![general(&app, &alex, s.id).await.id];
    for name in ["memes", "games", "music", "random"] {
        let r = post_json(
            &app,
            &alex,
            &format!("/servers/{}/channels", s.id),
            serde_json::json!({ "kind": "text", "name": name }),
        )
        .await;
        let c: pulse_protocol::rest::Channel = r.json().await.unwrap();
        channels.push(c.id);
    }

    let words = vocabulary();
    let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let started = Instant::now();
    let mut tx = app.db.begin().await.unwrap();
    for i in 0..MESSAGES {
        // Common words follow a rough power law; one message in 2,500 says "zebracorn".
        let n = 3 + (next() % 15) as usize;
        let mut text: Vec<&str> = (0..n)
            .map(|_| {
                let r = next() % 1_000;
                &*words[((r * r) / 1_000 % words.len() as u64) as usize]
            })
            .collect();
        if i % 2_500 == 0 {
            text.push("zebracorn");
        }
        let author = if i % 3 == 0 { sam_id } else { alex_id };
        sqlx::query(
            "INSERT INTO messages (id, channel_id, author_id, content, created_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(pulse_protocol::ids::MessageId::new().to_string())
        .bind(channels[i % channels.len()].to_string())
        .bind(author.to_string())
        .bind(text.join(" "))
        .bind(pulse_server::db::now())
        .execute(&mut *tx)
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
    println!("filled {MESSAGES} messages (indexed by the triggers) in {:?}", started.elapsed());

    let common = &words[0];
    // (query, typical): typical ones must meet the budget; the last one is printed only.
    let queries = [
        (format!("q={common}"), true),
        ("q=zebracorn".to_string(), true),
        (format!("q={common}+{}", words[1]), true),
        (format!("q={}", &common[..2]), true),
        (format!("from={sam_id}"), true),
        (format!("q={common}&from={sam_id}"), true),
        ("q=zebracorn&has=link".to_string(), true),
        // A filter-only search nothing matches walks every doc: the worst case, not a typical one.
        ("has=link".to_string(), false),
    ];
    let mut worst = Duration::ZERO;
    for (q, typical) in &queries {
        let path = format!("/servers/{}/search?{q}", s.id);
        let mut times = Vec::new();
        let mut hits = 0;
        for _ in 0..7 {
            let t = Instant::now();
            let r: SearchResults = get_json(&app, &alex, &path).await;
            times.push(t.elapsed());
            hits = r.total.unwrap_or(0);
        }
        times.sort();
        let median = times[times.len() / 2];
        let note = if *typical { "" } else { "  [worst case, not budgeted]" };
        println!("{q:<40} median {median:>10.2?}  (matches counted: {hits}){note}");
        if *typical {
            worst = worst.max(median);
        }
    }
    assert!(worst < BUDGET, "slowest typical query took {worst:?}");
}
