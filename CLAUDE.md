# Working on Pulse

## The backlog is GitHub Issues

Every feature, idea and bug that isn't done right now lives in this repository's GitHub Issues, and nowhere else.

- **Anything deferred gets an issue at the moment it's deferred.** When a design, plan, review, agent report or
  conversation says "later", "next round", "phase 2", "maybe", "out of scope for now" or "left for a decision", open
  an issue for it (or comment on the existing one) before the work that deferred it merges. A deferral that lives only
  in a spec or a plan is lost.
- Labels: `feature` or `bug`, one `area: …`, one `size: S|M|L`, plus `queued` (in the agreed build order) or `parked`
  (wanted some day). Point the body at the design that describes it (`docs/superpowers/specs/…:line`).
- Commits reference their issue (`#47`); the commit that finishes one says `Closes #47`.
- When a feature merges, check its design's non-goals and "later" notes against the issues.
- The repository is public: issues follow the same rules as commits (no real names, IPs, secrets or private
  details; test people are alex, sam, kim and jo).
