//! Every badge's definition and the pure rules that turn counters into tiers. Rarities and
//! thresholds are the spec's draft, to be tuned on the badge page.

use std::collections::HashMap;

/// Seconds in an hour: `Unit::Hours` thresholds are written as `10 * HOUR`.
pub const HOUR: i64 = 3600;

/// The `badge_stats` keys. Hours stats are stored in seconds.
pub mod stat {
    pub const MESSAGES: &str = "messages";
    pub const VOICE_SECS: &str = "voice_secs";
    pub const NIGHT_OWL: &str = "night_owl";
    pub const EARLY_BIRD_DAYS: &str = "early_bird_days";
    /// The South African day (days since the epoch) of the last Early Bird day counted.
    pub const EARLY_BIRD_LAST_DAY: &str = "early_bird_last_day";
    pub const GHOST: &str = "ghost";
    pub const DOOS: &str = "doos";
    /// Muted-talk rate limit state: the last counted report (unix seconds), its South African day
    /// and how many were counted that day.
    pub const DOOS_LAST_AT: &str = "doos_last_at";
    pub const DOOS_DAY: &str = "doos_day";
    pub const DOOS_DAY_COUNT: &str = "doos_day_count";
    pub const RENAMED_BY_OTHERS: &str = "renamed_by_others";
    /// Saved a Zalgo display name or server nickname (#210).
    pub const ZALGO_NAME: &str = "zalgo_name";
    pub const REACTIONS_RECEIVED: &str = "reactions_received";
    pub const EMOJI_SMITH: &str = "emoji_smith";
    /// `emoji_uses:<emoji id>`: uses by others of one of the uploader's emoji.
    pub const EMOJI_USES_PREFIX: &str = "emoji_uses:";
    pub const QUOTE_AWARDS: &str = "quote_awards";
    pub const QOTY_WINS: &str = "qoty_wins";
    pub const QUOTED_OTHERS: &str = "quoted_others";
    /// `vendetta:<user id>`: quotes of that one person.
    pub const VENDETTA_PREFIX: &str = "vendetta:";
    pub const VENDETTA_MAX: &str = "vendetta_max";
    pub const FRIENDLY_FIRE: &str = "friendly_fire";
    pub const CLIPS: &str = "clips";
    pub const HIGHLIGHT_REEL: &str = "highlight_reel";
    pub const STREAM_SECS: &str = "stream_secs";
    pub const LOGO_EGG: &str = "logo_egg";
    pub const EARLY_ADOPTER: &str = "early_adopter";

    /// Every fixed key (the prefixed ones excluded).
    pub const ALL: &[&str] = &[
        MESSAGES,
        VOICE_SECS,
        NIGHT_OWL,
        EARLY_BIRD_DAYS,
        EARLY_BIRD_LAST_DAY,
        GHOST,
        DOOS,
        DOOS_LAST_AT,
        DOOS_DAY,
        DOOS_DAY_COUNT,
        RENAMED_BY_OTHERS,
        ZALGO_NAME,
        REACTIONS_RECEIVED,
        EMOJI_SMITH,
        QUOTE_AWARDS,
        QOTY_WINS,
        QUOTED_OTHERS,
        VENDETTA_MAX,
        FRIENDLY_FIRE,
        CLIPS,
        HIGHLIGHT_REEL,
        STREAM_SECS,
        LOGO_EGG,
        EARLY_ADOPTER,
    ];
}

/// Ascending: `Mythical` is the rarest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rarity {
    Common,
    Uncommon,
    Rare,
    Epic,
    Legendary,
    Mythical,
}

impl Rarity {
    pub fn to_protocol(self) -> pulse_protocol::rest::BadgeRarity {
        use pulse_protocol::rest::BadgeRarity as P;
        match self {
            Self::Common => P::Common,
            Self::Uncommon => P::Uncommon,
            Self::Rare => P::Rare,
            Self::Epic => P::Epic,
            Self::Legendary => P::Legendary,
            Self::Mythical => P::Mythical,
        }
    }
}

/// How a badge's progress reads. `Hours` stats are stored in seconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Count,
    Hours,
}

/// How a badge is earned. Tiers are ascending thresholds, each with its own rarity.
#[derive(Debug)]
pub enum Rule {
    /// One stat against thresholds in its raw unit (seconds for `Unit::Hours`).
    Counter {
        stat: &'static str,
        tiers: &'static [(i64, Rarity)],
    },
    /// Messages, voice seconds and clips, all three at once.
    Combo {
        tiers: &'static [([i64; 3], Rarity)],
    },
    /// Distinct badges held, this one excluded.
    Collector { tiers: &'static [(i64, Rarity)] },
    /// Hidden badges held.
    HiddenCount { tiers: &'static [(i64, Rarity)] },
    /// Badges with two or more tiers held at their top tier, this one excluded.
    Perfectionist { tiers: &'static [(i64, Rarity)] },
    /// Only ever given by the owner with the admin command.
    HandGiven { rarity: Rarity },
    /// No trigger yet: only the admin command can give it.
    Never { rarity: Rarity },
}

#[derive(Debug)]
pub struct Def {
    pub key: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub icon: &'static str,
    /// Shown as "???" until earned.
    pub hidden: bool,
    /// Earned again and again; the stat is shown as a count (×2, ×3).
    pub stacking: bool,
    pub unit: Unit,
    pub rule: Rule,
}

impl Def {
    /// Each tier's threshold and rarity. `Combo` thresholds are the tier numbers (its real
    /// thresholds are triples); hand-given and never-earned badges have one tier `(1, rarity)`.
    pub fn tiers(&self) -> Vec<(i64, Rarity)> {
        match &self.rule {
            Rule::Counter { tiers, .. }
            | Rule::Collector { tiers }
            | Rule::HiddenCount { tiers }
            | Rule::Perfectionist { tiers } => tiers.to_vec(),
            Rule::Combo { tiers } => (1..).zip(tiers.iter()).map(|(n, (_, r))| (n, *r)).collect(),
            Rule::HandGiven { rarity } | Rule::Never { rarity } => vec![(1, *rarity)],
        }
    }

    pub fn max_tier(&self) -> u8 {
        self.tiers().len() as u8
    }

    /// The rarity of `tier`, clamped to the badge's tiers (tier 0 reads as tier 1).
    pub fn rarity_at(&self, tier: u8) -> Rarity {
        let tiers = self.tiers();
        let i = (tier.max(1) as usize).min(tiers.len()) - 1;
        tiers[i].1
    }

    /// The stat a counter badge reads.
    pub fn stat(&self) -> Option<&'static str> {
        match self.rule {
            Rule::Counter { stat, .. } => Some(stat),
            _ => None,
        }
    }

    /// Earned from other badges rather than from a stat.
    pub fn is_meta(&self) -> bool {
        matches!(
            self.rule,
            Rule::Collector { .. } | Rule::HiddenCount { .. } | Rule::Perfectionist { .. }
        )
    }

    /// Whether [`evaluate`] can award it (hand-given and never-earned badges it can't).
    pub fn earnable(&self) -> bool {
        !matches!(self.rule, Rule::HandGiven { .. } | Rule::Never { .. })
    }

    /// A raw stat value in the badge's unit (whole hours for `Unit::Hours`).
    pub fn shown(&self, raw: i64) -> u64 {
        let raw = raw.max(0) as u64;
        match self.unit {
            Unit::Count => raw,
            Unit::Hours => raw / HOUR as u64,
        }
    }

    /// Each tier's threshold as people read it: hours for `Unit::Hours`, and for `Combo` its
    /// message count (the first of its three; [`Def::parts`] gives all three for the next tier).
    pub fn thresholds_shown(&self) -> Vec<u64> {
        match &self.rule {
            Rule::Combo { tiers } => tiers.iter().map(|([m, ..], _)| *m as u64).collect(),
            _ => self
                .tiers()
                .into_iter()
                .map(|(t, _)| self.shown(t))
                .collect(),
        }
    }

    /// Someone's value towards this badge in its unit, from their `stats` and the badges they
    /// `held`. `None` where there's no one number: Combo, hand-given and never-earned badges.
    pub fn progress(
        &self,
        stats: &HashMap<String, i64>,
        held: &HashMap<&'static str, u8>,
    ) -> Option<u64> {
        match self.rule {
            Rule::Counter { stat, .. } => Some(self.shown(stats.get(stat).copied().unwrap_or(0))),
            _ => meta_value(self, held).map(|v| v.max(0) as u64),
        }
    }

    /// Combo's progress (#191): each of its three stats against the next tier not yet `held`,
    /// as `(label, value, target)`, hours whole. Empty for every other badge and once all are held.
    pub fn parts(
        &self,
        stats: &HashMap<String, i64>,
        held: &HashMap<&'static str, u8>,
    ) -> Vec<(&'static str, u64, u64)> {
        let Rule::Combo { tiers } = self.rule else {
            return Vec::new();
        };
        let have = held.get(self.key).copied().unwrap_or(0) as usize;
        let Some(([messages, voice, clips], _)) = tiers.get(have) else {
            return Vec::new();
        };
        let raw = |s: &str| stats.get(s).copied().unwrap_or(0).max(0) as u64;
        let hours = HOUR as u64;
        vec![
            ("Messages", raw(stat::MESSAGES), *messages as u64),
            (
                "Hours in voice",
                raw(stat::VOICE_SECS) / hours,
                *voice as u64 / hours,
            ),
            ("Clips", raw(stat::CLIPS), *clips as u64),
        ]
    }
}

use Rarity::*;

const fn counter(
    key: &'static str,
    name: &'static str,
    description: &'static str,
    icon: &'static str,
    stat: &'static str,
    tiers: &'static [(i64, Rarity)],
) -> Def {
    Def {
        key,
        name,
        description,
        icon,
        hidden: false,
        stacking: false,
        unit: Unit::Count,
        rule: Rule::Counter { stat, tiers },
    }
}

const fn hours(def: Def) -> Def {
    Def {
        unit: Unit::Hours,
        ..def
    }
}

const fn hidden(def: Def) -> Def {
    Def {
        hidden: true,
        ..def
    }
}

const fn stacking(def: Def) -> Def {
    Def {
        stacking: true,
        ..def
    }
}

/// Every badge the server can award, in trophy-case order.
pub static CATALOGUE: &[Def] = &[
    // Voice
    counter(
        "night_owl",
        "Night Owl",
        "In voice with someone at 3 am (South African time).",
        "🦉",
        stat::NIGHT_OWL,
        &[(1, Rare)],
    ),
    counter(
        "early_bird",
        "Early Bird",
        "First into voice in a server on 10 different days.",
        "🐦",
        stat::EARLY_BIRD_DAYS,
        &[(10, Uncommon)],
    ),
    counter(
        "ghost",
        "Ghost",
        "Joined and left voice within 5 seconds, 10 times.",
        "👻",
        stat::GHOST,
        &[(10, Rare)],
    ),
    hours(counter(
        "the_regular",
        "The Regular",
        "Hours in voice with at least one other person.",
        "🎧",
        stat::VOICE_SECS,
        &[
            (10 * HOUR, Common),
            (50 * HOUR, Uncommon),
            (100 * HOUR, Rare),
            (250 * HOUR, Epic),
            (500 * HOUR, Legendary),
            (1000 * HOUR, Mythical),
        ],
    )),
    counter(
        "the_doos",
        "The Doos",
        "Talked into a muted mic 10 times.",
        "🤐",
        stat::DOOS,
        &[(10, Epic)],
    ),
    // Names
    counter(
        "identity_crisis",
        "Identity Crisis",
        "Renamed by others.",
        "🎭",
        stat::RENAMED_BY_OTHERS,
        &[(10, Common), (25, Uncommon), (50, Epic), (100, Legendary)],
    ),
    // Its name is itself Zalgo (#210): a fixed, moderate one, three marks a letter, so the name
    // clipping (#209) shows it whole. Earned by saving a Zalgo name (`names::is_zalgo`).
    hidden(counter(
        "zalgo",
        "Z\u{335}\u{352}\u{316}a\u{334}\u{30d}\u{323}l\u{336}\u{346}\u{32d}g\u{337}\u{33e}\u{347}o\u{335}\u{352}\u{317}",
        "Saved a name in Zalgo. He comes.",
        "👁️",
        stat::ZALGO_NAME,
        &[(1, Rare)],
    )),
    // Chat
    counter(
        "the_yapper",
        "The Yapper",
        "Messages sent in servers.",
        "💬",
        stat::MESSAGES,
        &[
            (100, Common),
            (500, Uncommon),
            (1000, Rare),
            (5000, Epic),
            (10000, Legendary),
        ],
    ),
    counter(
        "reaction_farmer",
        "Reaction Farmer",
        "Reactions received from others.",
        "🌾",
        stat::REACTIONS_RECEIVED,
        &[(50, Rare), (100, Epic), (500, Legendary), (1000, Mythical)],
    ),
    counter(
        "emoji_smith",
        "Emoji Smith",
        "Added 5 custom emoji that others used 10 times each.",
        "⚒️",
        stat::EMOJI_SMITH,
        &[(5, Rare)],
    ),
    // Quotes
    counter(
        "quotable",
        "Quotable",
        "Quote awards received.",
        "📜",
        stat::QUOTE_AWARDS,
        &[
            (10, Common),
            (20, Uncommon),
            (30, Rare),
            (50, Epic),
            (100, Legendary),
        ],
    ),
    stacking(counter(
        "quote_of_the_year",
        "Quote of the Year",
        "Won Quote of the Year.",
        "🏆",
        stat::QOTY_WINS,
        &[(1, Legendary)],
    )),
    counter(
        "john_quoter",
        "John Quoter",
        "Quoted others 25 times.",
        "✍️",
        stat::QUOTED_OTHERS,
        &[(25, Uncommon)],
    ),
    counter(
        "personal_vendetta",
        "Personal Vendetta",
        "Quoted the same person 20 times.",
        "🎯",
        stat::VENDETTA_MAX,
        &[(20, Rare)],
    ),
    hidden(counter(
        "friendly_fire",
        "Friendly Fire",
        "Quoted yourself.",
        "🔫",
        stat::FRIENDLY_FIRE,
        &[(1, Uncommon)],
    )),
    // Clips and streams
    counter(
        "clipper",
        "Clipper",
        "Clips shared.",
        "🎬",
        stat::CLIPS,
        &[(5, Common), (25, Rare), (100, Epic)],
    ),
    hidden(counter(
        "highlight_reel",
        "Highlight Reel",
        "Bro is good at games: a clip with 5 reactions.",
        "🎞️",
        stat::HIGHLIGHT_REEL,
        &[(1, Uncommon)],
    )),
    hours(counter(
        "chat_did_you_see_that",
        "Chat, did you see that?",
        "Hours streamed.",
        "📺",
        stat::STREAM_SECS,
        &[
            (50 * HOUR, Uncommon),
            (200 * HOUR, Rare),
            (500 * HOUR, Epic),
            (1000 * HOUR, Legendary),
        ],
    )),
    // Meta and rare
    counter(
        "early_adopter",
        "Early Adopter",
        "Joined within the first week after launch.",
        "🌱",
        stat::EARLY_ADOPTER,
        &[(1, Epic)],
    ),
    Def {
        key: "badge_collector",
        name: "Badge Collector",
        description: "Badges earned.",
        icon: "🗃️",
        hidden: false,
        stacking: false,
        unit: Unit::Count,
        rule: Rule::Collector {
            tiers: &[(10, Rare), (20, Epic), (30, Legendary)],
        },
    },
    Def {
        key: "how_did_we_get_here",
        name: "How Did We Get Here?",
        description: "Unlocked 3 hidden badges.",
        icon: "🌀",
        hidden: false,
        stacking: false,
        unit: Unit::Count,
        // Three of the four hidden badges that can be earned (Skill Issue is only ever given).
        rule: Rule::HiddenCount {
            tiers: &[(3, Epic)],
        },
    },
    Def {
        key: "perfectionist",
        name: "Perfectionist",
        description: "Reached the top tier of 5 tiered badges.",
        icon: "💎",
        hidden: false,
        stacking: false,
        unit: Unit::Count,
        rule: Rule::Perfectionist {
            tiers: &[(5, Mythical)],
        },
    },
    Def {
        key: "combo",
        name: "Combo",
        // the tiers' numbers show one tier at a time as progress (#191)
        description: "Messages, hours in voice and clips, all at once.",
        icon: "🔗",
        hidden: false,
        stacking: false,
        unit: Unit::Count,
        rule: Rule::Combo {
            tiers: &[
                ([100, 100 * HOUR, 10], Uncommon),
                ([500, 500 * HOUR, 25], Epic),
                ([1000, 1000 * HOUR, 100], Mythical),
            ],
        },
    },
    hidden(counter(
        "logo_egg",
        "Egg Hunter",
        "Found the egg in the logo. Easter? Egg?",
        "🥚",
        stat::LOGO_EGG,
        &[(1, Epic)],
    )),
    Def {
        key: "skill_issue",
        name: "Skill Issue",
        description: "Did something spectacularly embarrassing.",
        icon: "🤡",
        hidden: true,
        stacking: false,
        unit: Unit::Count,
        rule: Rule::Never { rarity: Epic },
    },
];

/// One-of-a-kind badges the owner gives with the admin command; adding one is one entry here.
pub static HAND_GIVEN: &[Def] = &[
    Def {
        key: "founder",
        name: "Founder",
        description: "Made Pulse.",
        icon: "👑",
        hidden: false,
        stacking: false,
        unit: Unit::Count,
        rule: Rule::HandGiven { rarity: Mythical },
    },
    Def {
        key: "partner",
        name: "alienussy",
        description: "One of a kind, for a special someone.",
        // shown by clients without its picture (#225), and until one is uploaded
        icon: "💚",
        hidden: false,
        stacking: false,
        unit: Unit::Count,
        rule: Rule::HandGiven { rarity: Mythical },
    },
    Def {
        key: "patient_zero",
        name: "Patient Zero",
        description: "One of a kind: the first soul brave enough to test Pulse.",
        icon: "🧪",
        hidden: false,
        stacking: false,
        unit: Unit::Count,
        rule: Rule::HandGiven { rarity: Mythical },
    },
];

/// The definition of `key`, earnable or hand-given.
pub fn find(key: &str) -> Option<&'static Def> {
    CATALOGUE.iter().chain(HAND_GIVEN).find(|d| d.key == key)
}

/// How many of the ascending `thresholds` `value` reaches.
pub fn tier_for(value: i64, thresholds: impl Iterator<Item = i64>) -> u8 {
    thresholds.take_while(|t| value >= *t).count() as u8
}

/// The tier each earnable badge has reached from `stats`, never below its `current` tier; badges
/// at tier 0 are left out. Meta badges count everything held (hand-given included) and are
/// iterated until nothing changes, so earning ten badges at once also earns Badge Collector.
pub fn evaluate(
    stats: &HashMap<String, i64>,
    current: &HashMap<String, u8>,
) -> HashMap<&'static str, u8> {
    let stat = |k: &str| stats.get(k).copied().unwrap_or(0);
    let mut held: HashMap<&'static str, u8> = current
        .iter()
        .filter_map(|(k, t)| Some((find(k)?.key, *t)))
        .filter(|(_, t)| *t > 0)
        .collect();
    let raise = |held: &mut HashMap<&'static str, u8>, key: &'static str, tier: u8| {
        let t = held.entry(key).or_insert(0);
        let raised = tier > *t;
        *t = (*t).max(tier);
        raised
    };

    for def in CATALOGUE {
        let tier = match &def.rule {
            Rule::Counter { stat: s, tiers } => tier_for(stat(s), tiers.iter().map(|t| t.0)),
            Rule::Combo { tiers } => tiers
                .iter()
                .take_while(|([m, v, c], _)| {
                    stat(stat::MESSAGES) >= *m
                        && stat(stat::VOICE_SECS) >= *v
                        && stat(stat::CLIPS) >= *c
                })
                .count() as u8,
            _ => continue,
        };
        raise(&mut held, def.key, tier);
    }

    loop {
        let mut changed = false;
        for def in CATALOGUE.iter().filter(|d| d.is_meta()) {
            let value = meta_value(def, &held).expect("is_meta");
            let tier = tier_for(value, def.tiers().into_iter().map(|t| t.0));
            changed |= raise(&mut held, def.key, tier);
        }
        if !changed {
            break;
        }
    }

    held.retain(|k, t| *t > 0 && find(k).is_some_and(Def::earnable));
    held
}

/// What a meta badge counts over the badges `held` (itself excluded); `None` for the others.
pub fn meta_value(def: &Def, held: &HashMap<&'static str, u8>) -> Option<i64> {
    let others = || {
        held.iter()
            .filter(|(k, t)| **k != def.key && **t > 0)
            .filter_map(|(k, t)| Some((find(k)?, *t)))
    };
    let n = match def.rule {
        Rule::Collector { .. } => others().count(),
        Rule::HiddenCount { .. } => others().filter(|(d, _)| d.hidden).count(),
        Rule::Perfectionist { .. } => others()
            .filter(|(d, t)| d.max_tier() >= 2 && *t >= d.max_tier())
            .count(),
        _ => return None,
    };
    Some(n as i64)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn stats(pairs: &[(&str, i64)]) -> HashMap<String, i64> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect()
    }

    fn thresholds(def: &Def) -> Vec<i64> {
        def.tiers().into_iter().map(|(t, _)| t).collect()
    }

    fn rarities(def: &Def) -> Vec<Rarity> {
        (1..=def.max_tier()).map(|t| def.rarity_at(t)).collect()
    }

    #[test]
    fn tier_for_counts_reached_thresholds() {
        assert_eq!(tier_for(99, [100, 500].into_iter()), 0);
        assert_eq!(tier_for(100, [100, 500].into_iter()), 1);
        assert_eq!(
            tier_for(10_000, [100, 500, 1000, 5000, 10000].into_iter()),
            5
        );
    }

    #[test]
    fn the_regular_rarities_climb() {
        use Rarity::*;
        let def = find("the_regular").unwrap();
        assert_eq!(def.unit, Unit::Hours);
        assert_eq!(
            thresholds(def),
            [10, 50, 100, 250, 500, 1000].map(|h| h * HOUR)
        );
        assert_eq!(
            rarities(def),
            [Common, Uncommon, Rare, Epic, Legendary, Mythical]
        );
    }

    #[test]
    fn identity_crisis_tiers_are_10_25_50_100_c_u_e_l() {
        use Rarity::*;
        let def = find("identity_crisis").unwrap();
        assert_eq!(thresholds(def), [10, 25, 50, 100]);
        assert_eq!(rarities(def), [Common, Uncommon, Epic, Legendary]);
    }

    #[test]
    fn reaction_farmer_tiers_are_50_100_500_1000_r_e_l_m() {
        use Rarity::*;
        let def = find("reaction_farmer").unwrap();
        assert_eq!(thresholds(def), [50, 100, 500, 1000]);
        assert_eq!(rarities(def), [Rare, Epic, Legendary, Mythical]);
    }

    #[test]
    fn combo_parts_show_only_the_next_tier() {
        let def = find("combo").unwrap();
        let s = stats(&[
            (stat::MESSAGES, 120),
            (stat::VOICE_SECS, 42 * HOUR + 1800),
            (stat::CLIPS, 3),
        ]);
        let none = HashMap::new();
        assert_eq!(
            def.parts(&s, &none),
            [
                ("Messages", 120, 100),
                ("Hours in voice", 42, 100),
                ("Clips", 3, 10)
            ]
        );
        let one = HashMap::from([("combo", 1u8)]);
        assert_eq!(
            def.parts(&s, &one),
            [
                ("Messages", 120, 500),
                ("Hours in voice", 42, 500),
                ("Clips", 3, 25)
            ]
        );
        let all = HashMap::from([("combo", 3u8)]);
        assert!(def.parts(&s, &all).is_empty());
        assert!(find("the_yapper").unwrap().parts(&s, &none).is_empty());
    }

    #[test]
    fn combo_description_is_short() {
        assert_eq!(
            find("combo").unwrap().description,
            "Messages, hours in voice and clips, all at once."
        );
    }

    #[test]
    fn combo_needs_all_three() {
        let none = HashMap::new();
        let mut s = stats(&[
            (stat::MESSAGES, 1000),
            (stat::CLIPS, 100),
            (stat::VOICE_SECS, 99 * HOUR),
        ]);
        assert_eq!(evaluate(&s, &none).get("combo"), None);
        s.insert(stat::VOICE_SECS.into(), 100 * HOUR);
        assert_eq!(evaluate(&s, &none).get("combo"), Some(&1));
    }

    #[test]
    fn meta_badges_reach_a_fixed_point() {
        // Ten single-tier counter badges in one evaluation.
        let s = stats(&[
            (stat::NIGHT_OWL, 1),
            (stat::EARLY_BIRD_DAYS, 10),
            (stat::GHOST, 10),
            (stat::DOOS, 10),
            (stat::EMOJI_SMITH, 5),
            (stat::QOTY_WINS, 1),
            (stat::QUOTED_OTHERS, 25),
            (stat::VENDETTA_MAX, 20),
            (stat::FRIENDLY_FIRE, 1),
            (stat::HIGHLIGHT_REEL, 1),
        ]);
        let got = evaluate(&s, &HashMap::new());
        assert_eq!(got.get("badge_collector"), Some(&1));
    }

    #[test]
    fn skill_issue_and_hand_given_are_never_evaluated() {
        let s: HashMap<String, i64> = CATALOGUE
            .iter()
            .chain(HAND_GIVEN)
            .map(|d| (d.key.to_owned(), i64::MAX / 2))
            .chain(stat::ALL.iter().map(|k| ((*k).to_owned(), i64::MAX / 2)))
            .collect();
        let got = evaluate(&s, &HashMap::new());
        assert!(!got.is_empty());
        for key in ["skill_issue", "founder", "partner", "patient_zero"] {
            assert!(!got.contains_key(key), "{key} was evaluated");
        }
    }

    #[test]
    fn how_did_we_get_here_needs_the_three_earnable_hidden_badges() {
        use Rarity::*;
        let def = find("how_did_we_get_here").unwrap();
        assert_eq!(def.tiers(), [(3, Epic)]);
        let earnable_hidden = CATALOGUE
            .iter()
            .filter(|d| d.hidden && d.earnable())
            .count();
        // Four can be earned now (Zalgo joined, #210); any three will do.
        assert_eq!(earnable_hidden, 4);
        let mut s = stats(&[(stat::FRIENDLY_FIRE, 1), (stat::HIGHLIGHT_REEL, 1)]);
        assert_eq!(evaluate(&s, &HashMap::new()).get(def.key), None);
        s.insert(stat::LOGO_EGG.into(), 1);
        assert_eq!(evaluate(&s, &HashMap::new()).get(def.key), Some(&1));
    }

    #[test]
    fn evaluate_never_lowers_current() {
        let current = HashMap::from([("the_yapper".to_owned(), 3u8)]);
        let got = evaluate(&stats(&[(stat::MESSAGES, 0)]), &current);
        assert_eq!(got.get("the_yapper"), Some(&3));
    }

    #[test]
    fn keys_are_unique_and_hand_given_is_data() {
        let mut keys: Vec<_> = CATALOGUE.iter().chain(HAND_GIVEN).map(|d| d.key).collect();
        let n = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), n, "duplicate badge keys");
        let given: Vec<_> = HAND_GIVEN.iter().map(|d| d.key).collect();
        assert_eq!(given, ["founder", "partner", "patient_zero"]);
        assert!(HAND_GIVEN.iter().all(|d| !d.hidden && !d.earnable()));
        assert_eq!(find("partner").unwrap().max_tier(), 1);
    }

    #[test]
    fn the_one_of_a_kind_badges_are_named_and_keep_an_emoji() {
        use Rarity::*;
        let partner = find("partner").unwrap();
        assert_eq!(partner.name, "alienussy");
        assert_eq!(partner.description, "One of a kind, for a special someone.");
        assert_eq!(partner.icon, "💚");
        let zero = find("patient_zero").unwrap();
        assert_eq!(zero.name, "Patient Zero");
        assert_eq!(
            zero.description,
            "One of a kind: the first soul brave enough to test Pulse."
        );
        assert_eq!(zero.icon, "🧪");
        for def in HAND_GIVEN {
            assert_eq!(def.tiers(), [(1, Mythical)], "{}", def.key);
        }
    }

    #[test]
    fn hidden_set_is_zalgo_friendly_fire_highlight_reel_logo_egg_skill_issue() {
        let hidden: Vec<_> = CATALOGUE
            .iter()
            .chain(HAND_GIVEN)
            .filter(|d| d.hidden)
            .map(|d| d.key)
            .collect();
        assert_eq!(
            hidden,
            [
                "zalgo",
                "friendly_fire",
                "highlight_reel",
                "logo_egg",
                "skill_issue"
            ]
        );
    }

    #[test]
    fn the_zalgo_badge_is_hidden_one_off_and_named_in_zalgo() {
        use Rarity::*;
        let def = find("zalgo").unwrap();
        assert!(def.hidden && def.earnable() && !def.stacking);
        assert_eq!(def.stat(), Some(stat::ZALGO_NAME));
        assert_eq!(def.tiers(), [(1, Rare)]);
        assert_eq!(def.icon, "👁️");
        // Its name is Zalgo by the same rule that earns it, and a name anyone could choose.
        assert!(crate::profiles::names::is_zalgo(def.name), "{:?}", def.name);
        assert_eq!(crate::profiles::names::clean(def.name).unwrap(), def.name);
        assert!(def.name.chars().count() <= crate::profiles::names::NAME_MAX);
        // Moderate: the plain word underneath, at most three marks on any letter (#209's clip shows it).
        let base: String = def
            .name
            .chars()
            .filter(|c| !unicode_normalization::char::is_combining_mark(*c))
            .collect();
        assert_eq!(base, "Zalgo");
        assert!(
            def.name
                .split(|c: char| !unicode_normalization::char::is_combining_mark(c))
                .all(|marks| marks.chars().count() <= 3)
        );
        let s = stats(&[(stat::ZALGO_NAME, 1)]);
        assert_eq!(evaluate(&s, &HashMap::new()).get("zalgo"), Some(&1));
        assert_eq!(evaluate(&stats(&[]), &HashMap::new()).get("zalgo"), None);
    }
}
