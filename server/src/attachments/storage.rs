//! The `storage` command (#57, `just storage`): what the bucket holds by kind (message files,
//! temporary files, clips) and what's on disk (sounds, emoji), with R2's estimated monthly cost.

use sqlx::SqlitePool;

use crate::error::AppResult;

/// One line of the report.
#[derive(Debug, PartialEq)]
pub struct Kind {
    pub label: &'static str,
    pub count: u64,
    pub bytes: u64,
    /// In the bucket (counted towards its cost), else on the server's disk.
    pub r2: bool,
}

async fn sum(db: &SqlitePool, sql: &str) -> AppResult<(u64, u64)> {
    let (count, bytes): (i64, i64) = sqlx::query_as(sql).fetch_one(db).await?;
    Ok((count as u64, bytes as u64))
}

/// Every kind's files and bytes, from the database.
pub async fn usage(db: &SqlitePool) -> AppResult<Vec<Kind>> {
    let rows = [
        (
            "attachments",
            "SELECT COUNT(*), COALESCE(SUM(size), 0) FROM attachments WHERE expires_at IS NULL",
            true,
        ),
        (
            "temp",
            "SELECT COUNT(*), COALESCE(SUM(size), 0) FROM attachments
             WHERE expires_at IS NOT NULL AND purged_at IS NULL",
            true,
        ),
        (
            "clips",
            "SELECT COUNT(*), COALESCE(SUM(size), 0) FROM media WHERE kind = 'clip'",
            true,
        ),
        (
            "sounds",
            "SELECT COUNT(*), COALESCE(SUM(size), 0) FROM media WHERE kind = 'sound'",
            false,
        ),
        (
            "emoji",
            "SELECT COUNT(*), COALESCE(SUM(size), 0) FROM media WHERE kind = 'emoji'",
            false,
        ),
    ];
    let mut out = Vec::with_capacity(rows.len());
    for (label, sql, r2) in rows {
        let (count, bytes) = sum(db, sql).await?;
        out.push(Kind {
            label,
            count,
            bytes,
            r2,
        });
    }
    Ok(out)
}

/// The report: a line per kind, then the bucket's total and estimated cost.
pub fn format_report(kinds: &[Kind]) -> String {
    let gb = |bytes: u64| bytes as f64 / 1e9;
    let mut lines: Vec<String> = kinds
        .iter()
        .map(|k| {
            format!(
                "{:<12} {:>7} files  {:>8.2} GB  {}",
                k.label,
                k.count,
                gb(k.bytes),
                if k.r2 { "R2" } else { "disk" }
            )
        })
        .collect();
    let total = gb(kinds.iter().filter(|k| k.r2).map(|k| k.bytes).sum());
    let cost = (total - crate::clips::FREE_GB).max(0.0) * crate::clips::USD_PER_GB_MONTH;
    lines.push(format!(
        "R2 total {total:.2} GB  ~${cost:.2}/month (after R2's 10 GB free tier; the backups bucket shares it)"
    ));
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_lists_each_kind_and_the_bucket_cost() {
        let kinds = [
            Kind {
                label: "attachments",
                count: 3,
                bytes: 4_000_000_000,
                r2: true,
            },
            Kind {
                label: "temp",
                count: 1,
                bytes: 16_000_000_000,
                r2: true,
            },
            Kind {
                label: "clips",
                count: 2,
                bytes: 1_000_000_000,
                r2: true,
            },
            Kind {
                label: "sounds",
                count: 5,
                bytes: 50_000_000,
                r2: false,
            },
        ];
        let r = format_report(&kinds);
        let lines: Vec<&str> = r.lines().collect();
        assert_eq!(lines.len(), 5);
        assert!(lines[0].starts_with("attachments"), "{r}");
        assert!(
            lines[1].contains("16.00 GB") && lines[1].ends_with("R2"),
            "{r}"
        );
        assert!(lines[3].ends_with("disk"), "{r}");
        // 21 GB in the bucket, 11 past the free tier
        assert!(
            lines[4].starts_with("R2 total 21.00 GB  ~$0.16/month"),
            "{r}"
        );
    }
}
