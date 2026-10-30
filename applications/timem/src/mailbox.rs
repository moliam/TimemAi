//! Cross-instance session mailboxes.
//!
//! Every session owns exactly one mailbox whose name is globally unique on
//! this machine. Mail crosses timem process (and mem-directory) boundaries
//! through a shared user-level spool directory: a sender atomically drops a
//! JSON envelope into the recipient's directory, and the recipient's host
//! watcher injects the mail into the session's next model request (or starts
//! a fresh turn when idle). Plain filesystem semantics keep the mechanism
//! instance-agnostic; an inotify-backed watcher can replace the poll loop
//! later without changing the on-disk envelope format.

use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};

/// One mail envelope as it travels through the spool directory.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MailboxMail {
    pub id: String,
    pub from: String,
    pub to: String,
    pub msg: String,
    pub created_at_ms: u128,
    /// Max seconds the mail may wait unconsumed in the recipient's inbox
    /// before the recipient's runtime forces it into the next model request
    /// (the mail then counts as read); -1 waits indefinitely.
    #[serde(default = "default_queue_timeout", alias = "max_wait_secs")]
    pub queue_timeout_secs: i64,
    /// Sender-side deadline for the read receipt; -1 waits indefinitely.
    #[serde(default = "default_resp_timeout")]
    pub resp_timeout_secs: i64,
    /// "mail" (default) or "receipt" (delivery acknowledgement).
    #[serde(default = "default_mail_kind")]
    pub kind: String,
    /// For receipts: the id of the mail this receipt acknowledges.
    #[serde(default)]
    pub receipt_for: Option<String>,
    /// For receipts: `read` (legacy default), `timeout`, `discarded`, or
    /// `failed`. Keeping the status structured lets the sender apply
    /// idempotent ledger transitions instead of parsing display text.
    #[serde(default)]
    pub receipt_status: Option<String>,
}

fn default_queue_timeout() -> i64 {
    agent_core::host::MAILBOX_DEFAULT_QUEUE_TIMEOUT_SECS
}

fn default_resp_timeout() -> i64 {
    agent_core::host::MAILBOX_DEFAULT_RESP_TIMEOUT_SECS
}

fn default_mail_kind() -> String {
    "mail".to_string()
}

/// User-level spool root shared by every timem instance of this user: mail
/// crosses instance boundaries through the shared filesystem, not through
/// in-process channels.
pub fn mailbox_spool_root() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        if !home.trim().is_empty() {
            return Path::new(&home).join(".timem").join("mailboxes");
        }
    }
    std::env::temp_dir().join("timem-mailboxes")
}

pub fn mailbox_spool_dir(spool_root: &Path, mailbox_name: &str) -> PathBuf {
    spool_root.join(sanitize_mailbox_component(mailbox_name))
}

fn sanitize_mailbox_component(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Machine-locally unique, short and typeable: `mb-xxxxxx` (6 lowercase
/// base36 characters, 36^6 = 2,176,782,336 combinations). The scheduler
/// additionally retries when a generated name is already taken on this
/// machine, so collisions are practically impossible while the address stays
/// easy to read, say out loud and type.
pub fn generate_mailbox_name() -> String {
    const ALPHABET: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mix = nanos ^ (std::process::id() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let mut value = mix ^ (mix >> 32);
    let mut suffix = [0u8; 6];
    for slot in suffix.iter_mut() {
        *slot = ALPHABET[(value % ALPHABET.len() as u64) as usize];
        value /= ALPHABET.len() as u64;
    }
    format!("mb-{}", std::str::from_utf8(&suffix).unwrap_or("000000"))
}

/// Short mail id: `mail_` + 6 base36 chars hashed from (body, created_at_ms,
/// recipient mailbox, per-draw secure randomness). Every draw mixes fresh
/// entropy, so two instances hashing the same content for the same recipient
/// in the same millisecond still get different ids; senders additionally
/// retry on a spool or ledger collision, keeping ids practically unique
/// across instances while short and typeable.
pub fn generate_mail_id(msg: &str, created_at_ms: u128, mailbox_id: &str) -> String {
    const ALPHABET: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut hasher = DefaultHasher::new();
    msg.hash(&mut hasher);
    created_at_ms.hash(&mut hasher);
    mailbox_id.hash(&mut hasher);
    // Per-draw secure randomness: even identical (content, time, mailbox)
    // inputs on different instances produce different ids.
    let mut random = [0u8; 8];
    if agent_core::os::fill_secure_random(&mut random).is_err() {
        // Entropy fallback: nanos mixed with the process id.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        random =
            (nanos ^ (std::process::id() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)).to_le_bytes();
    }
    hasher.write(&random);
    let mut value = hasher.finish();
    let mut suffix = [0u8; 6];
    for slot in suffix.iter_mut() {
        *slot = ALPHABET[(value % ALPHABET.len() as u64) as usize];
        value /= ALPHABET.len() as u64;
    }
    format!("mail_{}", std::str::from_utf8(&suffix).unwrap_or("000000"))
}

/// Full spool envelope path for one mail id in one mailbox directory.
pub fn mail_envelope_path(spool_root: &Path, mailbox_name: &str, id: &str) -> PathBuf {
    mailbox_spool_dir(spool_root, mailbox_name).join(format!("mail_{id}.json"))
}

/// True when the recipient's spool already holds an envelope with this id.
pub fn spool_contains_mail(spool_root: &Path, mailbox_name: &str, id: &str) -> bool {
    mail_envelope_path(spool_root, mailbox_name, id).exists()
}

/// Picks a mailbox name whose spool directory is not taken yet, retrying a
/// bounded number of times before falling back to one more draw.
pub fn acquire_mailbox_name(spool_root: &Path) -> String {
    acquire_mailbox_name_with(spool_root, generate_mailbox_name)
}

fn acquire_mailbox_name_with(spool_root: &Path, mut generate: impl FnMut() -> String) -> String {
    for _ in 0..8 {
        let name = generate();
        if !mailbox_exists(spool_root, &name) {
            return name;
        }
    }
    generate()
}

pub fn mailbox_exists(spool_root: &Path, mailbox_name: &str) -> bool {
    mailbox_spool_dir(spool_root, mailbox_name).is_dir()
}

/// Atomically drops one mail envelope into the recipient's directory. The
/// rename is the delivery event: readers only ever observe complete files.
pub fn write_mail(spool_root: &Path, mail: &MailboxMail) -> Result<(), String> {
    let dir = mailbox_spool_dir(spool_root, &mail.to);
    fs::create_dir_all(&dir).map_err(|_| "mailbox_spool_create_failed".to_string())?;
    let bytes =
        serde_json::to_vec(mail).map_err(|_| "mailbox_mail_serialize_failed".to_string())?;
    let temporary = dir.join(format!(".mail_{}.tmp", mail.id));
    let final_path = mail_envelope_path(spool_root, &mail.to, &mail.id);
    {
        let mut file =
            fs::File::create(&temporary).map_err(|_| "mailbox_mail_write_failed".to_string())?;
        file.write_all(&bytes)
            .map_err(|_| "mailbox_mail_write_failed".to_string())?;
        file.sync_all().ok();
    }
    fs::rename(&temporary, &final_path).map_err(|_| "mailbox_mail_write_failed".to_string())
}

/// Reads every complete envelope currently sitting in one mailbox directory.
/// Corrupt or partial files are skipped so one bad drop cannot block a queue.
pub fn scan_mails(spool_dir: &Path) -> Vec<(PathBuf, MailboxMail)> {
    let Ok(entries) = fs::read_dir(spool_dir) else {
        return Vec::new();
    };
    let mut mails = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !name.starts_with("mail_") || !name.ends_with(".json") {
            continue;
        }
        let Ok(bytes) = fs::read(&path) else {
            continue;
        };
        if let Ok(mail) = serde_json::from_slice::<MailboxMail>(&bytes) {
            mails.push((path, mail));
        }
    }
    mails.sort_by(|left, right| {
        left.1
            .created_at_ms
            .cmp(&right.1.created_at_ms)
            .then_with(|| left.1.id.cmp(&right.1.id))
    });
    mails
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("timem_mailbox_test_{label}_{nanos}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample(id: &str, to: &str) -> MailboxMail {
        MailboxMail {
            id: id.to_string(),
            from: "mb-sender".to_string(),
            to: to.to_string(),
            msg: "hello".to_string(),
            created_at_ms: 1,
            queue_timeout_secs: 300,
            resp_timeout_secs: -1,
            kind: "mail".to_string(),
            receipt_for: None,
            receipt_status: None,
        }
    }

    #[test]
    fn mail_ids_are_short_base36_and_distinct_per_draw() {
        let id = generate_mail_id("hello", 1_700_000_000_000, "mb-target");
        assert!(id.starts_with("mail_"));
        assert_eq!(id.len(), 11, "mail_ plus 6 base36 chars keeps it typeable");
        assert!(
            id[5..]
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
            "{id}"
        );
        // Every draw mixes fresh randomness, so even the same
        // (content, time, mailbox) on another instance yields a new id.
        let redraw = generate_mail_id("hello", 1_700_000_000_000, "mb-target");
        assert_ne!(redraw, id);
        assert_ne!(
            generate_mail_id("hello", 1_700_000_000_001, "mb-target"),
            id
        );
        assert_ne!(generate_mail_id("hello", 1_700_000_000_000, "mb-other"), id);
        assert_ne!(
            generate_mail_id("hellp", 1_700_000_000_000, "mb-target"),
            id
        );
    }

    #[test]
    fn spool_collision_check_reports_existing_envelope_ids() {
        let root = temp_root("idcollide");
        write_mail(&root, &sample("a1", "mb-target")).unwrap();
        assert!(spool_contains_mail(&root, "mb-target", "a1"));
        assert!(!spool_contains_mail(&root, "mb-target", "a2"));
        // Spool directories are per-recipient: the same id in another
        // mailbox is a different envelope and never collides on disk.
        assert!(!spool_contains_mail(&root, "mb-other", "a1"));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn mail_round_trips_through_the_spool() {
        let root = temp_root("roundtrip");
        write_mail(&root, &sample("a1", "mb-target")).unwrap();
        let mails = scan_mails(&mailbox_spool_dir(&root, "mb-target"));
        assert_eq!(mails.len(), 1);
        assert_eq!(mails[0].1.id, "a1");
        assert_eq!(mails[0].1.msg, "hello");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn scan_skips_corrupt_envelopes_and_foreign_files() {
        let root = temp_root("corrupt");
        let dir = mailbox_spool_dir(&root, "mb-target");
        fs::create_dir_all(&dir).unwrap();
        write_mail(&root, &sample("a1", "mb-target")).unwrap();
        fs::write(dir.join("mail_bad.json"), b"{not json").unwrap();
        fs::write(dir.join("notes.txt"), b"ignored").unwrap();
        let mails = scan_mails(&dir);
        assert_eq!(mails.len(), 1);
        assert_eq!(mails[0].1.id, "a1");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn generated_names_are_short_prefixed_and_unique() {
        let first = generate_mailbox_name();
        assert!(first.starts_with("mb-"));
        assert_eq!(first.len(), 9, "mb- plus 6 base36 chars keeps it typeable");
        assert!(
            first[3..]
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
            "{first}"
        );
        let mut duplicate = false;
        for _ in 0..64 {
            duplicate |= generate_mailbox_name() == first;
        }
        assert!(!duplicate);
    }

    #[test]
    fn acquire_retries_taken_names_and_falls_back_gracefully() {
        let root = temp_root("acquire");
        std::fs::create_dir_all(mailbox_spool_dir(&root, "mb-taken")).unwrap();
        let mut calls = 0u32;
        let picked = acquire_mailbox_name_with(&root, || {
            calls += 1;
            if calls == 1 {
                "mb-taken".to_string()
            } else {
                "mb-free".to_string()
            }
        });
        assert_eq!(picked, "mb-free", "a taken name must be retried");
        assert_eq!(calls, 2);

        // Every draw taken: the last resort still returns a usable name.
        let picked = acquire_mailbox_name_with(&root, || "mb-taken".to_string());
        assert_eq!(picked, "mb-taken");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn receipt_kind_round_trips_and_legacy_envelopes_default_to_mail() {
        let root = temp_root("receipt");
        let mut receipt = sample("a1", "mb-target");
        receipt.kind = "receipt".to_string();
        receipt.receipt_for = Some("a0".to_string());
        receipt.receipt_status = Some("read".to_string());
        write_mail(&root, &receipt).unwrap();
        let mails = scan_mails(&mailbox_spool_dir(&root, "mb-target"));
        assert_eq!(mails.len(), 1);
        assert_eq!(mails[0].1.kind, "receipt");
        assert_eq!(mails[0].1.receipt_for.as_deref(), Some("a0"));
        assert_eq!(mails[0].1.receipt_status.as_deref(), Some("read"));

        // Envelopes written before the kind field existed keep mail semantics.
        let legacy_dir = mailbox_spool_dir(&root, "mb-legacy");
        fs::create_dir_all(&legacy_dir).unwrap();
        fs::write(
            legacy_dir.join("mail_legacy.json"),
            br#"{"id":"l1","from":"mb-s","to":"mb-legacy","msg":"hi","created_at_ms":1,"max_wait_secs":-1}"#,
        )
        .unwrap();
        let mails = scan_mails(&legacy_dir);
        assert_eq!(mails.len(), 1);
        assert_eq!(mails[0].1.kind, "mail");
        assert_eq!(mails[0].1.receipt_for, None);
        assert_eq!(mails[0].1.receipt_status, None);
        // Legacy envelopes carry only max_wait_secs: it maps to the queue
        // timeout, while the sender-side read deadline takes the default.
        assert_eq!(mails[0].1.queue_timeout_secs, -1);
        assert_eq!(
            mails[0].1.resp_timeout_secs,
            agent_core::host::MAILBOX_DEFAULT_RESP_TIMEOUT_SECS
        );
        fs::remove_dir_all(&root).ok();
    }
}
