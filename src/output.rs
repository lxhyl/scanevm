use colored::Colorize;
use tabled::{Table, Tabled};
use tabled::settings::{Style, Modify, object::Columns, Width};

pub fn format_eth(wei: &str, symbol: &str) -> String {
    let wei_val: u128 = wei.parse().unwrap_or(0);
    let eth = wei_val as f64 / 1e18;
    format!("{:.6} {}", eth, symbol)
}

pub fn format_token_amount(amount: &str, decimals: &str) -> String {
    let dec: u32 = decimals.parse().unwrap_or(18);
    if dec == 0 {
        return amount.to_string();
    }
    let val: u128 = amount.parse().unwrap_or(0);
    let divisor = 10u128.pow(dec.min(18));
    let result = val as f64 / divisor as f64;
    if dec <= 6 {
        format!("{:.prec$}", result, prec = dec as usize)
    } else {
        format!("{:.6}", result)
    }
}

pub fn format_gwei(wei: &str) -> String {
    let val: u64 = wei.parse().unwrap_or(0);
    let gwei = val as f64 / 1e9;
    format!("{:.2} Gwei", gwei)
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
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC", year, month, day, h, m, s)
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
        .map(|(k, v)| KvRow { field: k.to_string(), value: v.clone() })
        .collect();

    let mut table = Table::new(data);
    table.with(Style::rounded());
    println!("{}", table);
}

/// Print a table from tabled rows
pub fn print_table<T: Tabled>(rows: Vec<T>) {
    if rows.is_empty() {
        println!("{}", "No results found.".dimmed());
        return;
    }
    let mut table = Table::new(rows);
    table.with(Style::rounded());
    table.with(Modify::new(Columns::new(..)).with(Width::wrap(40)));
    println!("{}", table);
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
