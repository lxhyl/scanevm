use colored::Colorize;
use tabled::settings::peaker::PriorityMax;
use tabled::settings::{Style, Width};
use tabled::{Table, Tabled};

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
