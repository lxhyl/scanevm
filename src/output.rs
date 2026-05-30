use colored::Colorize;
use tabled::settings::peaker::PriorityMax;
use tabled::settings::{Style, Width};
use tabled::{Table, Tabled};

pub fn format_eth(wei: &str, symbol: &str) -> String {
    format!("{} {}", format_units(wei, 18, 6), symbol)
}

pub fn format_token_amount(amount: &str, decimals: &str) -> String {
    let dec: u32 = decimals.trim().parse().unwrap_or(18);
    // Show full precision for low-decimal tokens, otherwise cap the display at 6.
    let display = if dec <= 6 { dec as usize } else { 6 };
    format_units(amount, dec, display)
}

pub fn format_gwei(wei: &str) -> String {
    format!("{} Gwei", format_units(wei, 9, 2))
}

/// Format a base-unit integer string as a decimal with `decimals` implied
/// fraction digits, truncated to `display` fraction digits.
///
/// Uses exact `u128` integer arithmetic (no `f64`) so it is correct for tokens
/// with more than 18 decimals and for values above 2^53 where `f64` would lose
/// precision. Non-numeric input formats as zero.
fn format_units(value: &str, decimals: u32, display: usize) -> String {
    let v: u128 = value.trim().parse().unwrap_or(0);
    if decimals == 0 {
        return v.to_string();
    }
    // 10^39 overflows u128; clamp so we never panic on absurd decimal counts.
    let dec = decimals.min(38);
    let divisor = 10u128.pow(dec);
    let whole = v / divisor;
    let frac = v % divisor;
    if display == 0 {
        return whole.to_string();
    }
    // Zero-pad the fraction to `dec` digits, then keep `display` of them.
    let frac_full = format!("{frac:0width$}", width = dec as usize);
    let frac_shown = &frac_full[..display.min(frac_full.len())];
    format!("{whole}.{frac_shown}")
}

pub fn format_timestamp(unix: &str) -> String {
    let ts: i64 = unix.parse().unwrap_or(0);
    // Simple UTC formatting without extra deps
    // seconds since epoch → human readable
    let secs = ts as u64;
    let days_since_epoch = secs / 86400;
    let time_of_day = secs % 86400;
    let h = time_of_day / 3600;
    let m = (time_of_day % 3600) / 60;
    let s = time_of_day % 60;

    // Calculate date from days since epoch (1970-01-01)
    let (year, month, day) = days_to_date(days_since_epoch);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
        year, month, day, h, m, s
    )
}

fn days_to_date(days: u64) -> (u64, u64, u64) {
    let mut remaining = days;
    let mut year = 1970u64;
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        year += 1;
    }
    let months = if is_leap(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut month = 1u64;
    for &days_in_month in &months {
        if remaining < days_in_month {
            break;
        }
        remaining -= days_in_month;
        month += 1;
    }
    (year, month, remaining + 1)
}

fn is_leap(year: u64) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

pub fn truncate_hash(hash: &str) -> String {
    if hash.len() <= 12 {
        return hash.to_string();
    }
    format!("{}...{}", &hash[..8], &hash[hash.len() - 6..])
}

pub fn truncate_addr(addr: &str) -> String {
    if addr.len() <= 12 {
        return addr.to_string();
    }
    format!("{}...{}", &addr[..8], &addr[addr.len() - 4..])
}

pub fn print_json<T: serde::Serialize>(data: &T) {
    println!("{}", serde_json::to_string_pretty(data).unwrap_or_default());
}

pub fn print_kv_table(rows: &[(&str, String)]) {
    #[derive(Tabled)]
    struct KvRow {
        #[tabled(rename = "Field")]
        field: String,
        #[tabled(rename = "Value")]
        value: String,
    }
    let data: Vec<KvRow> = rows
        .iter()
        .map(|(k, v)| KvRow {
            field: k.to_string(),
            value: v.clone(),
        })
        .collect();

    let mut table = Table::new(data);
    table.with(Style::rounded());
    println!("{}", table);
}

/// Print a table from tabled rows.
///
/// Columns size to their content. Only when the table would overflow the
/// terminal do we wrap — shrinking the widest column first — so values like a
/// 42-char address are shown on one line whenever there's room. When stdout is
/// not a terminal (piped), nothing wraps.
pub fn print_table<T: Tabled>(rows: Vec<T>) {
    if rows.is_empty() {
        println!("{}", "No results found.".dimmed());
        return;
    }
    let mut table = Table::new(rows);
    table.with(Style::rounded());
    if let Some(width) = terminal_width() {
        table.with(Width::wrap(width).keep_words().priority::<PriorityMax>());
    }
    println!("{}", table);
}

fn terminal_width() -> Option<usize> {
    terminal_size::terminal_size().map(|(w, _)| w.0 as usize)
}

pub fn success(s: &str) -> String {
    s.green().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_eth() {
        assert_eq!(format_eth("1000000000000000000", "ETH"), "1.000000 ETH");
        assert_eq!(format_eth("0", "ETH"), "0.000000 ETH");
        assert_eq!(format_eth("1500000000000000000", "ETH"), "1.500000 ETH");
    }

    #[test]
    fn format_eth_keeps_precision_above_2_pow_53() {
        // 123456789012.345678901234567890 ETH — f64 would corrupt the trailing digits.
        assert_eq!(
            format_eth("123456789012345678901234567890", "ETH"),
            "123456789012.345678 ETH"
        );
    }

    #[test]
    fn format_token_amount_handles_more_than_18_decimals() {
        // Regression: the old `.min(18)` cap made this 1,000,000x too large.
        assert_eq!(
            format_token_amount("1000000000000000000000000", "24"),
            "1.000000"
        );
    }

    #[test]
    fn format_token_amount_zero_and_low_decimals() {
        assert_eq!(format_token_amount("42", "0"), "42");
        assert_eq!(format_token_amount("123456", "6"), "0.123456");
        assert_eq!(format_token_amount("bogus", "18"), "0.000000");
    }

    #[test]
    fn format_gwei_does_not_overflow() {
        // Old u64-based impl returned "0.00 Gwei" for values above u64::MAX.
        assert_eq!(
            format_gwei("99999999999999999999999999"),
            "99999999999999999.99 Gwei"
        );
        assert_eq!(format_gwei("1500000000"), "1.50 Gwei");
    }

    #[test]
    fn test_format_timestamp() {
        let s = format_timestamp("0");
        assert!(s.starts_with("1970-01-01"));
    }

    #[test]
    fn test_truncate_hash() {
        let h = "0x1234567890abcdef1234567890abcdef";
        let t = truncate_hash(h);
        assert!(t.contains("..."));
    }
}
