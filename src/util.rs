//! Small self-contained helpers: keccak-256, EIP-55 checksums, hex, RFC 3339
//! time, and safe on-disk paths for untrusted source paths.

// ---- keccak-256 (the original Keccak padding Ethereum uses, not SHA3) ----

const RC: [u64; 24] = [
    0x0000000000000001,
    0x0000000000008082,
    0x800000000000808a,
    0x8000000080008000,
    0x000000000000808b,
    0x0000000080000001,
    0x8000000080008081,
    0x8000000000008009,
    0x000000000000008a,
    0x0000000000000088,
    0x0000000080008009,
    0x000000008000000a,
    0x000000008000808b,
    0x800000000000008b,
    0x8000000000008089,
    0x8000000000008003,
    0x8000000000008002,
    0x8000000000000080,
    0x000000000000800a,
    0x800000008000000a,
    0x8000000080008081,
    0x8000000000008080,
    0x0000000080000001,
    0x8000000080008008,
];
const ROTC: [u32; 24] = [
    1, 3, 6, 10, 15, 21, 28, 36, 45, 55, 2, 14, 27, 41, 56, 8, 25, 43, 62, 18, 39, 61, 20, 44,
];
const PILN: [usize; 24] = [
    10, 7, 11, 17, 18, 3, 5, 16, 8, 21, 24, 4, 15, 23, 19, 13, 12, 2, 20, 14, 22, 9, 6, 1,
];
const RATE: usize = 136;

fn keccak_f(st: &mut [u64; 25]) {
    for rc in RC {
        // theta
        let mut bc = [0u64; 5];
        for (i, b) in bc.iter_mut().enumerate() {
            *b = st[i] ^ st[i + 5] ^ st[i + 10] ^ st[i + 15] ^ st[i + 20];
        }
        for i in 0..5 {
            let t = bc[(i + 4) % 5] ^ bc[(i + 1) % 5].rotate_left(1);
            for j in (0..25).step_by(5) {
                st[j + i] ^= t;
            }
        }
        // rho + pi
        let mut t = st[1];
        for i in 0..24 {
            let j = PILN[i];
            let next = st[j];
            st[j] = t.rotate_left(ROTC[i]);
            t = next;
        }
        // chi
        for j in (0..25).step_by(5) {
            let row = [st[j], st[j + 1], st[j + 2], st[j + 3], st[j + 4]];
            for i in 0..5 {
                st[j + i] = row[i] ^ (!row[(i + 1) % 5] & row[(i + 2) % 5]);
            }
        }
        // iota
        st[0] ^= rc;
    }
}

fn absorb(st: &mut [u64; 25], block: &[u8]) {
    for (i, lane) in block.chunks_exact(8).enumerate() {
        st[i] ^= u64::from_le_bytes(lane.try_into().expect("8-byte lane"));
    }
    keccak_f(st);
}

pub fn keccak256(data: &[u8]) -> [u8; 32] {
    let mut st = [0u64; 25];
    let mut blocks = data.chunks_exact(RATE);
    for b in &mut blocks {
        absorb(&mut st, b);
    }
    let rest = blocks.remainder();
    let mut last = [0u8; RATE];
    last[..rest.len()].copy_from_slice(rest);
    last[rest.len()] ^= 0x01;
    last[RATE - 1] ^= 0x80;
    absorb(&mut st, &last);
    let mut out = [0u8; 32];
    for (i, lane) in st.iter().take(4).enumerate() {
        out[i * 8..i * 8 + 8].copy_from_slice(&lane.to_le_bytes());
    }
    out
}

// ---- hex and addresses ----

/// Lowercase hex without a `0x` prefix.
pub fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(DIGITS[(b >> 4) as usize] as char);
        s.push(DIGITS[(b & 15) as usize] as char);
    }
    s
}

/// `0x` + keccak-256 of `data`, lowercase.
pub fn keccak_hex(data: &[u8]) -> String {
    format!("0x{}", to_hex(&keccak256(data)))
}

/// The EIP-55 mixed-case form of a `0x` + 40-hex address. Anything else is
/// returned unchanged.
pub fn checksum(address: &str) -> String {
    let lower = address.trim().to_ascii_lowercase();
    let Some(hex) = lower.strip_prefix("0x") else {
        return address.to_string();
    };
    if hex.len() != 40 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return address.to_string();
    }
    let hash = keccak256(hex.as_bytes());
    let mut out = String::with_capacity(42);
    out.push_str("0x");
    for (i, c) in hex.chars().enumerate() {
        let nibble = (hash[i / 2] >> if i % 2 == 0 { 4 } else { 0 }) & 15;
        out.push(if c.is_ascii_alphabetic() && nibble >= 8 {
            c.to_ascii_uppercase()
        } else {
            c
        });
    }
    out
}

// ---- bounded concurrency ----

/// Run `f` over `items` with at most `limit` futures in flight; results keep
/// the input order. (A small stand-in for `futures`' `buffered`, so the
/// current-thread runtime can overlap slow provider requests.)
pub async fn run_limited<T, F, Fut>(items: Vec<T>, limit: usize, f: F) -> Vec<Fut::Output>
where
    F: Fn(T) -> Fut,
    Fut: std::future::Future,
{
    use std::pin::Pin;
    use std::task::Poll;
    let n = items.len();
    let mut pending = items.into_iter().enumerate();
    let mut running: Vec<(usize, Pin<Box<Fut>>)> = Vec::new();
    let mut out: Vec<Option<Fut::Output>> = (0..n).map(|_| None).collect();
    for (i, t) in pending.by_ref().take(limit.max(1)) {
        running.push((i, Box::pin(f(t))));
    }
    std::future::poll_fn(|cx| {
        let mut k = 0;
        while k < running.len() {
            if let Poll::Ready(v) = running[k].1.as_mut().poll(cx) {
                let (i, _) = running.swap_remove(k);
                out[i] = Some(v);
                if let Some((j, t)) = pending.next() {
                    running.push((j, Box::pin(f(t))));
                }
                // Index k now holds another future (or k is past the end).
            } else {
                k += 1;
            }
        }
        if running.is_empty() {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    })
    .await;
    out.into_iter()
        .map(|o| o.expect("every future ran to completion"))
        .collect()
}

// ---- time ----

/// The current UTC time as RFC 3339 (`2026-11-25T06:59:41Z`).
pub fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    rfc3339(secs)
}

fn rfc3339(unix: u64) -> String {
    let (days, rem) = (unix / 86_400, unix % 86_400);
    let (y, m, d) = civil_from_days(days as i64);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Days since 1970-01-01 → (year, month, day), proleptic Gregorian.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

// ---- safe paths for untrusted source paths ----

/// Map an untrusted source path (from a provider) to a safe relative path:
/// backslashes become `/`, drive letters, leading `/`, `.` and NUL are
/// dropped, `..` becomes `__`, and components over 200 bytes are shortened
/// with a hash suffix. Never empty, never escapes the directory it's joined to.
pub fn sanitize_rel_path(original: &str) -> String {
    let mut s = original.replace('\\', "/").replace('\0', "");
    let b = s.as_bytes();
    if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        s = s[2..].to_string();
    }
    let parts: Vec<String> = s
        .split('/')
        .filter(|c| !c.is_empty() && *c != ".")
        .map(|c| {
            let c = if c == ".." { "__" } else { c };
            if c.len() <= 200 {
                return c.to_string();
            }
            let h = &to_hex(&keccak256(c.as_bytes()))[..8];
            let mut end = 180;
            while !c.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}~{h}", &c[..end])
        })
        .collect();
    if parts.is_empty() {
        "_".to_string()
    } else {
        parts.join("/")
    }
}

/// Unique on-disk paths for a list of source paths: sanitized, safe on a
/// case-insensitive filesystem, and never a file where another needs a
/// directory (clashes get `~N` before the extension, or `_conflicts/N/`).
pub fn assign_disk_paths(originals: &[&str]) -> Vec<String> {
    use std::collections::HashSet;
    let mut files: HashSet<String> = HashSet::new();
    let mut dirs: HashSet<String> = HashSet::new();
    let mut out = Vec::with_capacity(originals.len());
    for orig in originals {
        let base = sanitize_rel_path(orig);
        let mut candidate = base.clone();
        let mut n = 1;
        loop {
            let lc = candidate.to_lowercase();
            let comps: Vec<&str> = lc.split('/').collect();
            let prefixes: Vec<String> = (1..comps.len()).map(|k| comps[..k].join("/")).collect();
            let prefix_clash = prefixes.iter().any(|p| files.contains(p));
            if !(prefix_clash || files.contains(&lc) || dirs.contains(&lc)) {
                files.insert(lc);
                dirs.extend(prefixes);
                break;
            }
            n += 1;
            candidate = if prefix_clash || n > 1000 {
                // A parent directory is already a file: renaming the leaf can't help.
                format!("_conflicts/{n}/{base}")
            } else {
                match base.rfind('.') {
                    Some(dot) if !base[dot + 1..].contains('/') => {
                        format!("{}~{n}.{}", &base[..dot], &base[dot + 1..])
                    }
                    _ => format!("{base}~{n}"),
                }
            };
        }
        out.push(candidate);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keccak_known_vectors() {
        assert_eq!(
            keccak_hex(b""),
            "0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
        );
        assert_eq!(
            keccak_hex(b"abc"),
            "0x4e03657aea45a94fc7d47ba826c8d667c0d1e6e33a64a036ec44f58fa12d6c45"
        );
        // Event topics already used elsewhere in scanevm.
        assert_eq!(
            keccak_hex(b"DiamondCut((address,uint8,bytes4[])[],address,bytes)"),
            "0x8faa70878671ccd212d20771b795c50af8fd3ff6cf27f4bde57e5d4de0aeb673"
        );
        // Longer than one 136-byte block.
        let long = vec![b'a'; 300];
        assert_eq!(keccak256(&long).len(), 32);
        assert_ne!(keccak256(&long[..136]), keccak256(&long[..137]));
    }

    #[test]
    fn eip55_checksums() {
        // From the EIP-55 test vectors.
        for a in [
            "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed",
            "0xfB6916095ca1df60bB79Ce92cE3Ea74c37c5d359",
            "0xdbF03B407c01E7cD3CBea99509d93f8DDDC8C6FB",
            "0xD1220A0cf47c7B9Be7A2E6BA89F429762e7b9aDb",
        ] {
            assert_eq!(checksum(&a.to_lowercase()), a);
        }
        assert_eq!(checksum("not an address"), "not an address");
    }

    #[test]
    fn rfc3339_dates() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339(1_795_590_000), "2026-11-25T07:00:00Z");
    }

    #[test]
    fn untrusted_paths_stay_inside() {
        assert_eq!(sanitize_rel_path("../../etc/passwd"), "__/__/etc/passwd");
        assert_eq!(sanitize_rel_path("/abs/A.sol"), "abs/A.sol");
        assert_eq!(sanitize_rel_path("C:\\x\\.\\B.sol"), "x/B.sol");
        assert_eq!(sanitize_rel_path(""), "_");
        let long = format!("{}.sol", "a".repeat(300));
        assert!(sanitize_rel_path(&long).len() < 200);
    }

    #[tokio::test]
    async fn run_limited_keeps_order_and_bounds_concurrency() {
        use std::cell::Cell;
        use std::time::Duration;
        let live = Cell::new(0usize);
        let peak = Cell::new(0usize);
        let out = run_limited((0..10u64).collect(), 3, |i| {
            let (live, peak) = (&live, &peak);
            async move {
                live.set(live.get() + 1);
                peak.set(peak.get().max(live.get()));
                // Later items finish first, so completion order differs from input order.
                tokio::time::sleep(Duration::from_millis(10 * (10 - i))).await;
                live.set(live.get() - 1);
                i * 2
            }
        })
        .await;
        assert_eq!(out, (0..10u64).map(|i| i * 2).collect::<Vec<_>>());
        assert_eq!(peak.get(), 3);
        assert!(run_limited(Vec::<u8>::new(), 4, |x| async move { x })
            .await
            .is_empty());
    }

    #[test]
    fn disk_paths_are_unique_case_insensitively() {
        let p = assign_disk_paths(&["A.sol", "a.sol", "lib", "lib/B.sol"]);
        assert_eq!(p[0], "A.sol");
        assert_eq!(p[1], "a~2.sol");
        assert_eq!(p[2], "lib");
        assert!(p[3].starts_with("_conflicts/"));
    }
}
