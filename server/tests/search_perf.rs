//! Search speed on a friend-group-sized server: ~50k messages, most over five channels, the rest
//! over a DM, nine more servers and a dozen groups (so "everywhere", #331, walks ~65 channels).
//! Ignored by default (it takes a while to fill); run with
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

    // A quiet DM beside the busy server: one message in 50 (1k of the 50k).
    let dm = create_dm(&app, &alex, &[sam_id]).await.id;

    // Everywhere else alex reads (#331): nine more servers of five channels, a dozen groups and
    // notes. One message in ten lands among them.
    let (kim_id, _) = register(&app, "kim").await;
    let (jo_id, _) = register(&app, "jo").await;
    let mut elsewhere = Vec::new();
    for n in 0..9 {
        let x = create_server(&app, &alex, &format!("Other {n}")).await;
        elsewhere.push(general(&app, &alex, x.id).await.id);
        for name in ["a", "b", "c", "d"] {
            let r = post_json(
                &app,
                &alex,
                &format!("/servers/{}/channels", x.id),
                serde_json::json!({ "kind": "text", "name": name }),
            )
            .await;
            let c: pulse_protocol::rest::Channel = r.json().await.unwrap();
            elsewhere.push(c.id);
        }
    }
    for n in 0..12 {
        let who = match n % 3 {
            0 => vec![sam_id, kim_id],
            1 => vec![sam_id, jo_id],
            _ => vec![kim_id, jo_id],
        };
        elsewhere.push(create_dm(&app, &alex, &who).await.id);
    }
    elsewhere.push(create_dm(&app, &alex, &[alex_id]).await.id);

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
        .bind(
            if i % 50 == 7 {
                dm
            } else if i % 10 == 3 {
                elsewhere[(i / 10) % elsewhere.len()]
            } else {
                channels[i % channels.len()]
            }
            .to_string(),
        )
        .bind(author.to_string())
        .bind(text.join(" "))
        .bind(pulse_server::db::now())
        .execute(&mut *tx)
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
    println!(
        "filled {MESSAGES} messages (indexed by the triggers) in {:?}",
        started.elapsed()
    );

    let common = &words[0];
    let server = |q: &str| format!("/servers/{}/search?{q}", s.id);
    let in_dm = |q: &str| format!("/channels/{dm}/search?{q}");
    let all = |q: &str| format!("/search?{q}");
    let main = s.id;
    let general = channels[0];
    // (path, typical): typical ones must meet the budget; the others are printed only.
    let queries = [
        (server(&format!("q={common}")), true),
        (server("q=zebracorn"), true),
        (server(&format!("q={common}+{}", words[1])), true),
        (server(&format!("q={}", &common[..2])), true),
        (server(&format!("from={sam_id}")), true),
        (server(&format!("q={common}&from={sam_id}")), true),
        (server("q=zebracorn&has=link"), true),
        (server(&format!("q=zebracorn&in={general}")), true),
        (server(&format!("q={common}&in={general}")), true),
        (in_dm(&format!("q={common}")), true),
        (in_dm("q=zebracorn"), true),
        (in_dm(&format!("from={sam_id}")), true),
        (in_dm("has=link"), true),
        // Everywhere (#331): ~65 channels as a filter on the same walk.
        (all(&format!("q={common}")), true),
        (all("q=zebracorn"), true),
        (all(&format!("q={common}+{}", words[1])), true),
        (all(&format!("q={}", &common[..2])), true),
        (all(&format!("from={sam_id}")), true),
        (all(&format!("q={common}&from={sam_id}")), true),
        (all("q=zebracorn&has=link"), true),
        // The switcher's second section from inside the busy server: everything but it.
        (all(&format!("q={common}&except_server={main}")), true),
        (all(&format!("q=zebracorn&except_server={main}")), true),
        (all(&format!("q={common}&except_channel={dm}")), true),
        // A filter-only search nothing matches walks every doc: the worst case, not a typical one.
        (server("has=link"), false),
        (all("has=link"), false),
    ];

    // The plan of an everywhere search with words (what push_from_where builds for several
    // channels): the index must lead, walked in rowid order, with no sort (the early stop).
    let mut list = vec![general.to_string(), dm.to_string()];
    list.extend(
        channels[1..]
            .iter()
            .chain(&elsewhere)
            .map(|c| c.to_string()),
    );
    let marks = vec!["?"; list.len()].join(", ");
    let sql = format!(
        "EXPLAIN QUERY PLAN SELECT s.rowid, m.id FROM message_search s
         CROSS JOIN search_docs d ON d.doc = s.rowid
         CROSS JOIN messages m ON m.id = d.message_id
         CROSS JOIN channels c ON c.id = m.channel_id
         WHERE message_search MATCH ? AND +d.channel_id IN ({marks})
         ORDER BY s.rowid DESC LIMIT 26"
    );
    let mut q = sqlx::query_as::<_, (i64, i64, i64, String)>(&sql).bind(common.clone());
    for c in &list {
        q = q.bind(c.clone());
    }
    let plan = q.fetch_all(&app.db).await.unwrap();
    println!("everywhere plan ({} channels):", list.len());
    for (_, _, _, step) in &plan {
        println!("  {step}");
    }
    assert!(
        plan[0].3.contains("VIRTUAL TABLE INDEX"),
        "the index leads: {plan:?}"
    );
    assert!(
        !plan.iter().any(|p| p.3.contains("TEMP B-TREE")),
        "no sort, so the walk stops after a page: {plan:?}"
    );

    let mut worst = Duration::ZERO;
    for (path, typical) in &queries {
        let mut times = Vec::new();
        let mut hits = 0;
        for _ in 0..7 {
            let t = Instant::now();
            let r: SearchResults = get_json(&app, &alex, path).await;
            times.push(t.elapsed());
            hits = r.total.unwrap_or(0);
        }
        times.sort();
        let median = times[times.len() / 2];
        let note = if *typical {
            ""
        } else {
            "  [worst case, not budgeted]"
        };
        let q = if path.starts_with("/channels") {
            format!("dm {}", &path[path.find('?').unwrap()..])
        } else if path.starts_with("/search") {
            format!("everywhere {}", &path[path.find('?').unwrap()..])
        } else {
            format!("server {}", &path[path.find('?').unwrap()..])
        };
        println!("{q:<70} median {median:>10.2?}  (matches counted: {hits}){note}");
        if *typical {
            worst = worst.max(median);
        }
    }
    assert!(worst < BUDGET, "slowest typical query took {worst:?}");
}
