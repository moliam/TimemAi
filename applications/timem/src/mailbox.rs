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
use std::fs;
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
    /// Max seconds the mail may wait unconsumed in the recipient's queue
    /// before forced delivery; -1 waits indefinitely.
    pub max_wait_secs: i64,
    /// "mail" (default) or "receipt" (delivery acknowledgement).
    #[serde(default = "default_mail_kind")]
    pub kind: String,
    /// For receipts: the id of the mail this receipt acknowledges.
    #[serde(default)]
    pub receipt_for: Option<String>,
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

/// Globally unique, short and typeable: `mb-xxxxxxxx`.
pub fn generate_mailbox_name() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mix = nanos ^ (std::process::id() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let left = (mix >> 32) as u32;
    let right = mix as u32;
    format!("mb-{left:04x}{right:04x}")
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
    let final_path = dir.join(format!("mail_{}.json", mail.id));
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
            max_wait_secs: 300,
            kind: "mail".to_string(),
            receipt_for: None,
        }
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
    fn generated_names_are_prefixed_and_unique() {
        let first = generate_mailbox_name();
        assert!(first.starts_with("mb-"));
        let mut duplicate = false;
        for _ in 0..64 {
            duplicate |= generate_mailbox_name() == first;
        }
        assert!(!duplicate);
    }
}
